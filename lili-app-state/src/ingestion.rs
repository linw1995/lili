use lili_session::{
    CodexAdapterDiagnostics, ForwardingAck, ForwardingAckDisposition, ForwardingCredentials,
    ForwardingProtocolError, ForwardingVerifier, NormalizedSessionEvent, ReductionOutcome,
    SpoolMetrics,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot, watch};

use crate::{AppState, ViewSnapshot};

pub const DEFAULT_INGESTION_QUEUE_CAPACITY: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionCategory {
    Authentication,
    Replay,
    Expired,
    Malformed,
    Transport,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestionDiagnostics {
    pub accepted_messages: u64,
    pub duplicate_events: u64,
    pub authentication_rejections: u64,
    pub replay_rejections: u64,
    pub expired_rejections: u64,
    pub malformed_rejections: u64,
    pub transport_rejections: u64,
    pub spool_expired_drops: u64,
    pub spool_limit_drops: u64,
    pub spool_malformed_drops: u64,
    pub codex_adapter: CodexAdapterDiagnostics,
}

impl IngestionDiagnostics {
    fn record_rejection(&mut self, category: RejectionCategory) {
        let counter = match category {
            RejectionCategory::Authentication => &mut self.authentication_rejections,
            RejectionCategory::Replay => &mut self.replay_rejections,
            RejectionCategory::Expired => &mut self.expired_rejections,
            RejectionCategory::Malformed => &mut self.malformed_rejections,
            RejectionCategory::Transport => &mut self.transport_rejections,
        };
        *counter = counter.saturating_add(1);
    }
}

#[derive(Clone)]
pub struct NativeIngestionHandle {
    sender: mpsc::Sender<IngestionCommand>,
    snapshots: watch::Receiver<ViewSnapshot>,
}

impl NativeIngestionHandle {
    pub async fn ingest(
        &self,
        payload: Vec<u8>,
        now_ms: u64,
    ) -> Result<ForwardingAck, IngestionError> {
        let (response_sender, response_receiver) = oneshot::channel();
        self.sender
            .send(IngestionCommand::Ingest {
                payload,
                now_ms,
                response: response_sender,
            })
            .await
            .map_err(|_| IngestionError::Unavailable)?;
        response_receiver
            .await
            .map_err(|_| IngestionError::Unavailable)?
    }

    pub async fn record_transport_rejection(&self) -> Result<(), IngestionError> {
        self.sender
            .send(IngestionCommand::TransportRejected)
            .await
            .map_err(|_| IngestionError::Unavailable)
    }

    pub async fn ingest_spooled(
        &self,
        event: NormalizedSessionEvent,
    ) -> Result<ReductionOutcome, IngestionError> {
        let (response_sender, response_receiver) = oneshot::channel();
        self.sender
            .send(IngestionCommand::IngestSpooled {
                event,
                response: response_sender,
            })
            .await
            .map_err(|_| IngestionError::Unavailable)?;
        response_receiver
            .await
            .map_err(|_| IngestionError::Unavailable)?
    }

    pub async fn set_spool_metrics(&self, metrics: SpoolMetrics) -> Result<(), IngestionError> {
        self.sender
            .send(IngestionCommand::SetSpoolMetrics(metrics))
            .await
            .map_err(|_| IngestionError::Unavailable)
    }

    pub fn subscribe(&self) -> watch::Receiver<ViewSnapshot> {
        self.snapshots.clone()
    }
}

pub struct NativeIngestionActor {
    state: AppState,
    verifier: ForwardingVerifier,
    receiver: mpsc::Receiver<IngestionCommand>,
    snapshot_sender: watch::Sender<ViewSnapshot>,
    diagnostics: IngestionDiagnostics,
}

impl NativeIngestionActor {
    pub async fn channel(
        state: AppState,
        credentials: ForwardingCredentials,
        capacity: usize,
    ) -> (NativeIngestionHandle, Self) {
        let capacity = capacity.max(1);
        let (sender, receiver) = mpsc::channel(capacity);
        let (snapshot_sender, snapshots) = watch::channel(state.snapshot().await);
        let actor = Self {
            state,
            verifier: ForwardingVerifier::new(credentials),
            receiver,
            snapshot_sender,
            diagnostics: IngestionDiagnostics::default(),
        };
        (NativeIngestionHandle { sender, snapshots }, actor)
    }

    pub async fn run(mut self) {
        while let Some(command) = self.receiver.recv().await {
            match command {
                IngestionCommand::Ingest {
                    payload,
                    now_ms,
                    response,
                } => {
                    let result = self.ingest(&payload, now_ms).await;
                    let _ = response.send(result);
                }
                IngestionCommand::TransportRejected => {
                    self.diagnostics
                        .record_rejection(RejectionCategory::Transport);
                    self.publish_diagnostics().await;
                }
                IngestionCommand::IngestSpooled { event, response } => {
                    let outcome = if event.validate().is_ok() {
                        Ok(self.reduce_event(event).await)
                    } else {
                        self.diagnostics
                            .record_rejection(RejectionCategory::Malformed);
                        self.publish_diagnostics().await;
                        Err(IngestionError::InvalidEvent)
                    };
                    let _ = response.send(outcome);
                }
                IngestionCommand::SetSpoolMetrics(metrics) => {
                    self.diagnostics.spool_expired_drops = metrics.expired_drops;
                    self.diagnostics.spool_limit_drops = metrics.limit_drops;
                    self.diagnostics.spool_malformed_drops = metrics.malformed_drops;
                    self.publish_diagnostics().await;
                }
            }
        }
    }

    async fn ingest(
        &mut self,
        payload: &[u8],
        now_ms: u64,
    ) -> Result<ForwardingAck, IngestionError> {
        let verified = match self.verifier.verify_payload(payload, now_ms) {
            Ok(verified) => verified,
            Err(error) => {
                self.diagnostics.record_rejection(rejection_category(error));
                self.publish_diagnostics().await;
                return Err(IngestionError::Protocol(error));
            }
        };
        let outcome = self.reduce_event(verified.event().clone()).await;
        let disposition = match outcome {
            ReductionOutcome::Applied { .. } | ReductionOutcome::IgnoredStale => {
                ForwardingAckDisposition::Accepted
            }
            ReductionOutcome::Duplicate => ForwardingAckDisposition::Duplicate,
        };
        Ok(verified.acknowledgement(disposition))
    }

    async fn reduce_event(&mut self, event: NormalizedSessionEvent) -> ReductionOutcome {
        self.diagnostics.codex_adapter.record_accepted_event(&event);
        let outcome = self.state.apply_session_event(event).await;
        match outcome {
            ReductionOutcome::Applied { .. } => {
                self.diagnostics.accepted_messages =
                    self.diagnostics.accepted_messages.saturating_add(1);
                let _ = self.snapshot_sender.send(self.state.snapshot().await);
            }
            ReductionOutcome::Duplicate => {
                self.diagnostics.duplicate_events =
                    self.diagnostics.duplicate_events.saturating_add(1);
            }
            ReductionOutcome::IgnoredStale => {
                self.diagnostics.accepted_messages =
                    self.diagnostics.accepted_messages.saturating_add(1);
            }
        }
        self.publish_diagnostics().await;
        outcome
    }

    async fn publish_diagnostics(&self) {
        self.state
            .replace_ingestion_diagnostics(self.diagnostics.clone())
            .await;
    }
}

enum IngestionCommand {
    Ingest {
        payload: Vec<u8>,
        now_ms: u64,
        response: oneshot::Sender<Result<ForwardingAck, IngestionError>>,
    },
    TransportRejected,
    IngestSpooled {
        event: NormalizedSessionEvent,
        response: oneshot::Sender<Result<ReductionOutcome, IngestionError>>,
    },
    SetSpoolMetrics(SpoolMetrics),
}

fn rejection_category(error: ForwardingProtocolError) -> RejectionCategory {
    match error {
        ForwardingProtocolError::InvalidMac | ForwardingProtocolError::WrongInstance => {
            RejectionCategory::Authentication
        }
        ForwardingProtocolError::ReplayedNonce => RejectionCategory::Replay,
        ForwardingProtocolError::Expired => RejectionCategory::Expired,
        ForwardingProtocolError::TooLarge
        | ForwardingProtocolError::Malformed
        | ForwardingProtocolError::UnsupportedVersion(_)
        | ForwardingProtocolError::InvalidEndpoint
        | ForwardingProtocolError::InvalidInstanceId
        | ForwardingProtocolError::InvalidCredential
        | ForwardingProtocolError::InvalidNonce
        | ForwardingProtocolError::MismatchedAcknowledgement
        | ForwardingProtocolError::InvalidEvent
        | ForwardingProtocolError::Randomness => RejectionCategory::Malformed,
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum IngestionError {
    #[error("native ingestion is unavailable")]
    Unavailable,
    #[error("native ingestion rejected the forwarding message: {0}")]
    Protocol(#[from] ForwardingProtocolError),
    #[error("native ingestion rejected an invalid normalized event")]
    InvalidEvent,
}

#[cfg(test)]
mod tests {
    use lili_session::{
        CodexIntegrationSurface, ProviderCapabilitiesInputV1, ProviderInputV1,
        normalize_provider_input,
    };

    use super::*;

    fn event() -> lili_session::NormalizedSessionEvent {
        normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("turn_completed".to_owned()),
            event_id: Some("event-1".to_owned()),
            session_id: Some("session-1".to_owned()),
            turn_id: Some("turn-1".to_owned()),
            occurred_at_ms: Some(10),
            project: None,
            summary: None,
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: Some("notify:codex-tui".to_owned()),
        })
        .unwrap()
    }

    fn payload(frame: &[u8]) -> Vec<u8> {
        let length = u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize;
        assert_eq!(length, frame.len() - 4);
        frame[4..].to_vec()
    }

    #[tokio::test]
    async fn actor_reduces_duplicate_event_once_and_publishes_one_revision() {
        let state = AppState::default();
        let credentials = ForwardingCredentials::generate().unwrap();
        let (handle, actor) = NativeIngestionActor::channel(
            state.clone(),
            credentials.clone(),
            DEFAULT_INGESTION_QUEUE_CAPACITY,
        )
        .await;
        let mut snapshots = handle.subscribe();
        let task = tokio::spawn(actor.run());

        let first = credentials.sign(event(), 1_000).unwrap();
        let second = credentials.sign(event(), 1_001).unwrap();
        assert_eq!(
            handle
                .ingest(payload(&first.to_frame().unwrap()), 1_000)
                .await
                .unwrap()
                .disposition(),
            ForwardingAckDisposition::Accepted
        );
        snapshots.changed().await.unwrap();
        assert_eq!(snapshots.borrow().revision, 1);
        assert_eq!(
            handle
                .ingest(payload(&second.to_frame().unwrap()), 1_001)
                .await
                .unwrap()
                .disposition(),
            ForwardingAckDisposition::Duplicate
        );
        assert_eq!(state.snapshot().await.revision, 1);
        let diagnostics = state.ingestion_diagnostics().await;
        assert_eq!(diagnostics.accepted_messages, 1);
        assert_eq!(diagnostics.duplicate_events, 1);
        assert_eq!(
            diagnostics.codex_adapter.discovered_surfaces,
            [CodexIntegrationSurface::Notify]
        );
        assert_eq!(
            diagnostics
                .codex_adapter
                .last_accepted_event
                .as_ref()
                .unwrap()
                .event_id,
            "event-1"
        );

        drop(handle);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn forged_message_is_rejected_without_state_change_or_payload_retention() {
        let state = AppState::default();
        let credentials = ForwardingCredentials::generate().unwrap();
        let attacker = ForwardingCredentials::generate().unwrap();
        let (handle, actor) = NativeIngestionActor::channel(state.clone(), credentials, 1).await;
        let task = tokio::spawn(actor.run());
        let forged = attacker.sign(event(), 1_000).unwrap();
        assert!(matches!(
            handle
                .ingest(payload(&forged.to_frame().unwrap()), 1_000)
                .await,
            Err(IngestionError::Protocol(
                ForwardingProtocolError::WrongInstance
            ))
        ));
        assert_eq!(state.snapshot().await.revision, 0);
        assert_eq!(
            state
                .ingestion_diagnostics()
                .await
                .authentication_rejections,
            1
        );

        drop(handle);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn trusted_spooled_event_is_reduced_before_claim_commit() {
        let state = AppState::default();
        let credentials = ForwardingCredentials::generate().unwrap();
        let (handle, actor) = NativeIngestionActor::channel(state.clone(), credentials, 1).await;
        let task = tokio::spawn(actor.run());
        assert_eq!(
            handle.ingest_spooled(event()).await.unwrap(),
            ReductionOutcome::Applied { revision: 1 }
        );
        assert_eq!(state.snapshot().await.revision, 1);

        drop(handle);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn failure_injection_before_reducer_acceptance_preserves_state() {
        let state = AppState::default();
        let credentials = ForwardingCredentials::generate().unwrap();
        let (handle, actor) = NativeIngestionActor::channel(state.clone(), credentials, 1).await;
        let task = tokio::spawn(actor.run());
        let mut invalid = event();
        invalid.version = lili_session::SESSION_SCHEMA_VERSION + 1;

        assert_eq!(
            handle.ingest_spooled(invalid).await,
            Err(IngestionError::InvalidEvent)
        );
        assert_eq!(state.snapshot().await.revision, 0);
        assert_eq!(state.ingestion_diagnostics().await.malformed_rejections, 1);

        drop(handle);
        task.await.unwrap();
    }
}
