mod ingestion;
mod persistence;

use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use lili_actions::{
    ActionAuditEntry, ActionExecutionOutcome, ActionExecutionResult, ActionSummary,
    ActionSupervisor, DisplayEventSummaryV1, EffectiveActionsView, InteractionContextV1,
    InteractionTrigger, LoadedActions, NotificationFilterKind, NotificationSnapshotV1,
    PetLifecycleSnapshotV1, PetSnapshotV1,
};
use lili_core::{
    PetActionFeedbackKind, PetActionFeedbackPresentation, PetLifecycleState, PetNotificationKind,
    PetNotificationPresentation, PetPresentationState,
};
use lili_pet::{AtlasFormat, PetCatalog, PetSummary};
use lili_session::{
    NormalizedSessionEvent, Notification, NotificationId, NotificationKind, NotificationState,
    PresentationState, ReductionOutcome, SessionEventKind, SessionReducer, SessionViewSnapshot,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock, watch};
use uuid::Uuid;

pub use ingestion::{
    DEFAULT_INGESTION_QUEUE_CAPACITY, IngestionDiagnostics, IngestionError, NativeIngestionActor,
    NativeIngestionHandle, RejectionCategory,
};
pub use persistence::{
    AppStateStore, DEFAULT_VISIBLE_WINDOW_MARGIN, DisplayWorkArea, PersistenceError,
    PersistentApplicationState, ResolvedWindowPlacement, WindowPlacement, resolve_window_placement,
};

const DISPATCH_HISTORY_LIMIT: usize = 2_048;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ViewSnapshot {
    pub revision: u64,
    pub selected_pet: Option<PetSummary>,
    pub pet_asset_id: Option<String>,
    pub session_state: SessionViewSnapshot,
    pub actions: Vec<ActionSummary>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserSettings {
    pub always_on_top: bool,
    pub reduced_motion: bool,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            always_on_top: true,
            reduced_motion: false,
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    snapshot: Arc<RwLock<ViewSnapshot>>,
    settings: Arc<RwLock<UserSettings>>,
    session_reducer: Arc<Mutex<SessionReducer>>,
    pet_catalog: Arc<RwLock<PetCatalog>>,
    pet_asset: Arc<RwLock<ApprovedPetAsset>>,
    ingestion_diagnostics: Arc<RwLock<IngestionDiagnostics>>,
    action_feedback: Arc<RwLock<Option<PetActionFeedbackPresentation>>>,
    action_runtime: Arc<RwLock<ActionRuntimeState>>,
    dispatched_interactions: Arc<Mutex<DispatchHistory>>,
    presentation_sender: Arc<watch::Sender<PetPresentationState>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InteractionDispatchReceipt {
    pub accepted: bool,
    pub action_count: usize,
}

#[derive(Clone, Default)]
struct ActionRuntimeState {
    supervisor: Option<ActionSupervisor>,
    effective: EffectiveActionsView,
}

#[derive(Default)]
struct DispatchHistory {
    order: VecDeque<Uuid>,
    identities: HashSet<Uuid>,
}

impl DispatchHistory {
    fn accept(&mut self, interaction_id: Uuid) -> bool {
        if !self.identities.insert(interaction_id) {
            return false;
        }
        self.order.push_back(interaction_id);
        if self.order.len() > DISPATCH_HISTORY_LIMIT
            && let Some(expired) = self.order.pop_front()
        {
            self.identities.remove(&expired);
        }
        true
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovedPetAsset {
    id: String,
    content_type: &'static str,
    bytes: Arc<[u8]>,
}

impl ApprovedPetAsset {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub const fn content_type(&self) -> &'static str {
        self.content_type
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl AppState {
    pub fn with_pet_catalog(pet_catalog: PetCatalog) -> Self {
        Self::with_reducer(pet_catalog, SessionReducer::default())
    }

    pub fn with_persistent_state(
        pet_catalog: PetCatalog,
        state: PersistentApplicationState,
    ) -> Result<Self, lili_session::ReducerRestoreError> {
        let reducer = SessionReducer::from_persistent_state(state.into_reducer_state())?;
        Ok(Self::with_reducer(pet_catalog, reducer))
    }

    fn with_reducer(pet_catalog: PetCatalog, reducer: SessionReducer) -> Self {
        let (pet_catalog, pet_asset) = load_active_asset(pet_catalog);
        let selected_pet = Some(PetSummary::from(pet_catalog.active().definition()));
        let pet_asset_id = Some(pet_asset.id().to_owned());
        let session_state = reducer.snapshot();
        let initial_snapshot = ViewSnapshot {
            revision: session_state.revision,
            selected_pet,
            pet_asset_id,
            session_state,
            actions: Vec::new(),
        };
        let (presentation_sender, _) =
            watch::channel(presentation_from_view(&initial_snapshot, false, None));
        Self {
            snapshot: Arc::new(RwLock::new(initial_snapshot)),
            settings: Arc::new(RwLock::new(UserSettings::default())),
            session_reducer: Arc::new(Mutex::new(reducer)),
            pet_catalog: Arc::new(RwLock::new(pet_catalog)),
            pet_asset: Arc::new(RwLock::new(pet_asset)),
            ingestion_diagnostics: Arc::new(RwLock::new(IngestionDiagnostics::default())),
            action_feedback: Arc::new(RwLock::new(None)),
            action_runtime: Arc::new(RwLock::new(ActionRuntimeState::default())),
            dispatched_interactions: Arc::new(Mutex::new(DispatchHistory::default())),
            presentation_sender: Arc::new(presentation_sender),
        }
    }

    pub async fn available_pets(&self) -> Vec<PetSummary> {
        self.pet_catalog.read().await.available_summaries()
    }

    pub async fn approved_pet_asset(&self, asset_id: &str) -> Option<ApprovedPetAsset> {
        let asset = self.pet_asset.read().await;
        (asset.id() == asset_id).then(|| asset.clone())
    }

    pub async fn replace_pet_catalog(&self, pet_catalog: PetCatalog) -> PetSummary {
        let (pet_catalog, pet_asset) = load_active_asset(pet_catalog);
        let selected_pet = PetSummary::from(pet_catalog.active().definition());
        let pet_asset_id = pet_asset.id().to_owned();
        *self.pet_catalog.write().await = pet_catalog;
        *self.pet_asset.write().await = pet_asset;
        {
            let mut snapshot = self.snapshot.write().await;
            snapshot.selected_pet = Some(selected_pet.clone());
            snapshot.pet_asset_id = Some(pet_asset_id);
        }
        self.publish_presentation().await;
        selected_pet
    }

    pub async fn snapshot(&self) -> ViewSnapshot {
        let mut snapshot = self.snapshot.read().await.clone();
        snapshot.session_state = self.session_reducer.lock().await.snapshot();
        snapshot
    }

    pub async fn pet_presentation(&self) -> PetPresentationState {
        let snapshot = self.snapshot().await;
        presentation_from_view(
            &snapshot,
            self.settings.read().await.reduced_motion,
            self.action_feedback.read().await.clone(),
        )
    }

    pub fn subscribe_pet_presentation(&self) -> watch::Receiver<PetPresentationState> {
        self.presentation_sender.subscribe()
    }

    pub async fn settings(&self) -> UserSettings {
        self.settings.read().await.clone()
    }

    pub async fn replace_settings(&self, settings: UserSettings) -> UserSettings {
        *self.settings.write().await = settings.clone();
        self.publish_presentation().await;
        settings
    }

    pub async fn apply_session_event(&self, event: NormalizedSessionEvent) -> ReductionOutcome {
        let starts_activity_reminder = starts_activity_reminder(event.event_type);
        let accepted_at_ms = unix_time_ms();
        let outcome = self
            .session_reducer
            .lock()
            .await
            .reduce_at(event, accepted_at_ms);
        if matches!(outcome, ReductionOutcome::Applied { .. }) {
            self.publish_presentation().await;
            if starts_activity_reminder {
                self.schedule_activity_reminder_expiry(accepted_at_ms);
            }
        }
        outcome
    }

    pub async fn apply_session_event_persisted(
        &self,
        event: NormalizedSessionEvent,
        store: &AppStateStore,
    ) -> Result<ReductionOutcome, PersistenceError> {
        let starts_activity_reminder = starts_activity_reminder(event.event_type);
        let accepted_at_ms = unix_time_ms();
        let selected_pet_id =
            lili_core::PetId::parse(self.pet_catalog.read().await.requested_identifier());
        let mut reducer = self.session_reducer.lock().await;
        let previous = reducer.clone();
        let outcome = reducer.reduce_at(event.clone(), accepted_at_ms);
        if matches!(outcome, ReductionOutcome::Applied { .. }) {
            let persistent =
                PersistentApplicationState::new(selected_pet_id, None, reducer.persistent_state());
            if let Err(error) = store.save(&persistent) {
                *reducer = previous;
                return Err(error);
            }
        }
        drop(reducer);
        if matches!(outcome, ReductionOutcome::Applied { .. }) {
            self.publish_presentation().await;
            if starts_activity_reminder {
                self.schedule_activity_reminder_expiry(accepted_at_ms);
            }
        }
        Ok(outcome)
    }

    pub async fn advance_presentation(&self, now_ms: u64) -> ReductionOutcome {
        let outcome = self
            .session_reducer
            .lock()
            .await
            .advance_presentation(now_ms);
        if matches!(outcome, ReductionOutcome::Applied { .. }) {
            self.publish_presentation().await;
        }
        outcome
    }

    pub async fn acknowledge_notification(
        &self,
        id: &NotificationId,
        now_ms: u64,
    ) -> ReductionOutcome {
        let outcome = self
            .session_reducer
            .lock()
            .await
            .acknowledge_notification(id, now_ms);
        if matches!(outcome, ReductionOutcome::Applied { .. }) {
            self.publish_presentation().await;
        }
        outcome
    }

    pub async fn acknowledge_notification_persisted(
        &self,
        id: &NotificationId,
        now_ms: u64,
        store: &AppStateStore,
    ) -> Result<ReductionOutcome, PersistenceError> {
        let selected_pet_id =
            lili_core::PetId::parse(self.pet_catalog.read().await.requested_identifier());
        let mut reducer = self.session_reducer.lock().await;
        let previous = reducer.clone();
        let outcome = reducer.acknowledge_notification(id, now_ms);
        if matches!(outcome, ReductionOutcome::Applied { .. }) {
            let persistent =
                PersistentApplicationState::new(selected_pet_id, None, reducer.persistent_state());
            if let Err(error) = store.save(&persistent) {
                *reducer = previous;
                return Err(error);
            }
        }
        drop(reducer);
        if matches!(outcome, ReductionOutcome::Applied { .. }) {
            self.publish_presentation().await;
        }
        Ok(outcome)
    }

    pub async fn notification_context(&self, id: &NotificationId) -> Option<Notification> {
        self.session_reducer
            .lock()
            .await
            .snapshot()
            .notifications
            .into_iter()
            .find(|notification| {
                notification.id == *id && notification.state == NotificationState::Unread
            })
    }

    pub async fn bind_interaction(
        &self,
        interaction_id: Uuid,
        accepted_at_ms: u64,
        trigger: InteractionTrigger,
        notification_id: Option<&NotificationId>,
    ) -> Option<InteractionContextV1> {
        let reducer = self.session_reducer.lock().await;
        let session_snapshot = reducer.snapshot();
        let pet = self
            .interaction_pet_snapshot(session_snapshot.presentation)
            .await;
        match trigger {
            InteractionTrigger::NotificationActivate => {
                let notification_id = notification_id?;
                let notification =
                    session_snapshot
                        .notifications
                        .into_iter()
                        .find(|notification| {
                            notification.id == *notification_id
                                && notification.state == NotificationState::Unread
                        })?;
                Some(InteractionContextV1::for_notification(
                    interaction_id,
                    accepted_at_ms,
                    pet,
                    notification_snapshot(notification),
                ))
            }
            InteractionTrigger::PetClick | InteractionTrigger::PetDoubleClick => {
                InteractionContextV1::for_pet(interaction_id, accepted_at_ms, trigger, pet)
            }
        }
    }

    pub async fn publish_action_result(&self, result: &ActionExecutionResult) -> bool {
        let feedback = match result.outcome {
            ActionExecutionOutcome::Succeeded => PetActionFeedbackPresentation {
                action_id: result.action_id.clone(),
                kind: PetActionFeedbackKind::Success,
                message: "Action completed".to_owned(),
                occurred_at_ms: result.finished_at_ms,
            },
            ActionExecutionOutcome::Saturated => PetActionFeedbackPresentation {
                action_id: result.action_id.clone(),
                kind: PetActionFeedbackKind::Busy,
                message: "Action is busy".to_owned(),
                occurred_at_ms: result.finished_at_ms,
            },
            ActionExecutionOutcome::NonZeroExit => {
                action_failure(result, "Action exited with an error")
            }
            ActionExecutionOutcome::SpawnFailed => action_failure(result, "Action could not start"),
            ActionExecutionOutcome::IoFailed => action_failure(result, "Action I/O failed"),
            ActionExecutionOutcome::TimedOut => action_failure(result, "Action timed out"),
            ActionExecutionOutcome::OutputOverflow => {
                action_failure(result, "Action output limit exceeded")
            }
            ActionExecutionOutcome::Debounced
            | ActionExecutionOutcome::NotMatched
            | ActionExecutionOutcome::UnknownAction => return false,
        };
        *self.action_feedback.write().await = Some(feedback);
        self.publish_presentation().await;
        true
    }

    pub async fn configure_actions(
        &self,
        loaded: LoadedActions,
        global_concurrency: usize,
    ) -> bool {
        let effective = loaded.effective().clone();
        let summaries = loaded.summaries();
        let supervisor = ActionSupervisor::new(loaded, global_concurrency);
        let configured = supervisor.is_some();
        *self.action_runtime.write().await = ActionRuntimeState {
            supervisor,
            effective,
        };
        self.snapshot.write().await.actions = summaries;
        self.publish_presentation().await;
        configured
    }

    pub async fn effective_actions(&self) -> EffectiveActionsView {
        self.action_runtime.read().await.effective.clone()
    }

    pub async fn action_audit(&self) -> Vec<ActionAuditEntry> {
        let supervisor = self.action_runtime.read().await.supervisor.clone();
        match supervisor {
            Some(supervisor) => supervisor.audit_snapshot().await,
            None => Vec::new(),
        }
    }

    pub async fn dispatch_interaction(
        &self,
        context: InteractionContextV1,
    ) -> InteractionDispatchReceipt {
        if !self
            .dispatched_interactions
            .lock()
            .await
            .accept(context.interaction_id)
        {
            return InteractionDispatchReceipt {
                accepted: false,
                action_count: 0,
            };
        }
        let supervisor = self.action_runtime.read().await.supervisor.clone();
        let Some(supervisor) = supervisor else {
            return InteractionDispatchReceipt {
                accepted: true,
                action_count: 0,
            };
        };
        let action_ids = supervisor.matching_action_ids(&context);
        let action_count = action_ids.len();
        for action_id in action_ids {
            let state = self.clone();
            let supervisor = supervisor.clone();
            let context = context.clone();
            tokio::spawn(async move {
                let result = supervisor.execute(&action_id, &context).await;
                state.publish_action_result(&result).await;
            });
        }
        InteractionDispatchReceipt {
            accepted: true,
            action_count,
        }
    }

    pub async fn persistent_state(
        &self,
        window_placement: Option<WindowPlacement>,
    ) -> PersistentApplicationState {
        let selected_pet_id =
            lili_core::PetId::parse(self.pet_catalog.read().await.requested_identifier());
        PersistentApplicationState::new(
            selected_pet_id,
            window_placement,
            self.session_reducer.lock().await.persistent_state(),
        )
    }

    pub async fn ingestion_diagnostics(&self) -> IngestionDiagnostics {
        self.ingestion_diagnostics.read().await.clone()
    }

    pub(crate) async fn replace_ingestion_diagnostics(&self, diagnostics: IngestionDiagnostics) {
        *self.ingestion_diagnostics.write().await = diagnostics;
    }

    async fn publish_presentation(&self) {
        let session_state = self.session_reducer.lock().await.snapshot();
        let reduced_motion = self.settings.read().await.reduced_motion;
        let action_feedback = self.action_feedback.read().await.clone();
        let presentation = {
            let mut snapshot = self.snapshot.write().await;
            snapshot.session_state = session_state;
            snapshot.revision = snapshot.revision.saturating_add(1);
            presentation_from_view(&snapshot, reduced_motion, action_feedback)
        };
        self.presentation_sender.send_replace(presentation);
    }

    fn schedule_activity_reminder_expiry(&self, occurred_at_ms: u64) {
        let state = self.clone();
        let deadline_ms =
            occurred_at_ms.saturating_add(lili_session::DEFAULT_ACTIVITY_REMINDER_DURATION_MS);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(
                lili_session::DEFAULT_ACTIVITY_REMINDER_DURATION_MS,
            ))
            .await;
            let _ = state.advance_presentation(deadline_ms).await;
        });
    }

    async fn interaction_pet_snapshot(&self, presentation: PresentationState) -> PetSnapshotV1 {
        let snapshot = self.snapshot.read().await;
        PetSnapshotV1 {
            pet_id: snapshot
                .selected_pet
                .as_ref()
                .map_or_else(|| "unknown".to_owned(), |pet| pet.id.as_str().to_owned()),
            label: snapshot
                .selected_pet
                .as_ref()
                .map_or_else(|| "Desktop pet".to_owned(), |pet| pet.display_name.clone()),
            lifecycle: match presentation {
                PresentationState::Idle => PetLifecycleSnapshotV1::Idle,
                PresentationState::ActivityReminder => PetLifecycleSnapshotV1::ActivityReminder,
                PresentationState::Review => PetLifecycleSnapshotV1::Review,
                PresentationState::Failed => PetLifecycleSnapshotV1::Failed,
                PresentationState::Waiting => PetLifecycleSnapshotV1::Waiting,
            },
        }
    }
}

fn starts_activity_reminder(event_type: SessionEventKind) -> bool {
    matches!(
        event_type,
        SessionEventKind::SessionStarted
            | SessionEventKind::TurnStarted
            | SessionEventKind::AttentionResolved
    )
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().try_into().unwrap_or(u64::MAX)
        })
}

fn action_failure(
    result: &ActionExecutionResult,
    message: &'static str,
) -> PetActionFeedbackPresentation {
    PetActionFeedbackPresentation {
        action_id: result.action_id.clone(),
        kind: PetActionFeedbackKind::Failure,
        message: message.to_owned(),
        occurred_at_ms: result.finished_at_ms,
    }
}

fn notification_snapshot(notification: Notification) -> NotificationSnapshotV1 {
    NotificationSnapshotV1 {
        notification_id: notification.id.as_str().to_owned(),
        event_id: notification.event_id.as_str().to_owned(),
        provider: notification.provider.as_str().to_owned(),
        session_id: notification.session_id.as_str().to_owned(),
        turn_id: notification
            .turn_id
            .map(|turn_id| turn_id.as_str().to_owned()),
        kind: match notification.kind {
            NotificationKind::Attention => NotificationFilterKind::Attention,
            NotificationKind::Completion => NotificationFilterKind::Completion,
            NotificationKind::Failure => NotificationFilterKind::Failure,
        },
        occurred_at_ms: notification.occurred_at_ms,
        project_label: notification
            .project
            .map(|project| project.label().to_owned()),
        summary: notification.summary.map(|summary| DisplayEventSummaryV1 {
            text: summary.text().to_owned(),
            truncated: summary.was_truncated(),
            redacted: summary.was_redacted(),
        }),
    }
}

fn presentation_from_view(
    snapshot: &ViewSnapshot,
    reduced_motion: bool,
    action_feedback: Option<PetActionFeedbackPresentation>,
) -> PetPresentationState {
    let notifications = snapshot
        .session_state
        .notifications
        .iter()
        .filter(|notification| notification.state == NotificationState::Unread)
        .map(|notification| PetNotificationPresentation {
            activation_id: notification.id.as_str().to_owned(),
            kind: match notification.kind {
                NotificationKind::Attention => PetNotificationKind::Attention,
                NotificationKind::Completion => PetNotificationKind::Completion,
                NotificationKind::Failure => PetNotificationKind::Failure,
            },
            project_label: notification
                .project
                .as_ref()
                .map(|project| project.label().to_owned()),
            summary: notification.summary.as_ref().map_or_else(
                || match notification.kind {
                    NotificationKind::Attention => "Input required".to_owned(),
                    NotificationKind::Completion => "Task completed".to_owned(),
                    NotificationKind::Failure => "Task failed".to_owned(),
                },
                |summary| summary.text().to_owned(),
            ),
            summary_truncated: notification
                .summary
                .as_ref()
                .is_some_and(|summary| summary.was_truncated()),
            summary_redacted: notification
                .summary
                .as_ref()
                .is_some_and(|summary| summary.was_redacted()),
            occurred_at_ms: notification.occurred_at_ms,
            unread: true,
        })
        .collect::<Vec<_>>();
    PetPresentationState {
        revision: snapshot.revision,
        lifecycle: match snapshot.session_state.presentation {
            PresentationState::Idle => PetLifecycleState::Idle,
            PresentationState::ActivityReminder => PetLifecycleState::ActivityReminder,
            PresentationState::Review => PetLifecycleState::Review,
            PresentationState::Failed => PetLifecycleState::Failed,
            PresentationState::Waiting => PetLifecycleState::Waiting,
        },
        pet_asset_id: snapshot.pet_asset_id.clone(),
        pet_label: snapshot
            .selected_pet
            .as_ref()
            .map_or_else(|| "Desktop pet".to_owned(), |pet| pet.display_name.clone()),
        unread_notification_count: notifications.len(),
        notifications,
        action_feedback,
        reduced_motion,
    }
}

fn load_active_asset(pet_catalog: PetCatalog) -> (PetCatalog, ApprovedPetAsset) {
    if let Ok(asset) = approved_asset(&pet_catalog) {
        return (pet_catalog, asset);
    }

    let fallback = PetCatalog::default();
    let asset = approved_asset(&fallback).expect("embedded fallback asset must remain valid");
    (fallback, asset)
}

fn approved_asset(
    pet_catalog: &PetCatalog,
) -> Result<ApprovedPetAsset, lili_pet::AtlasValidationError> {
    let loaded = pet_catalog.active().load_asset()?;
    let content_type = match loaded.atlas().format() {
        AtlasFormat::Png => "image/png",
        AtlasFormat::WebP => "image/webp",
    };
    Ok(ApprovedPetAsset {
        id: Uuid::new_v4().simple().to_string(),
        content_type,
        bytes: Arc::from(loaded.bytes()),
    })
}

impl Default for AppState {
    fn default() -> Self {
        Self::with_pet_catalog(PetCatalog::default())
    }
}

#[cfg(test)]
mod tests {
    use lili_session::{
        ProviderCapabilitiesInputV1, ProviderInputV1, ProviderProjectInputV1, SessionPhase,
        normalize_provider_input,
    };
    use lili_storage::ApplicationPaths;

    use super::*;

    #[cfg(unix)]
    fn test_actions(source: &str) -> LoadedActions {
        lili_actions::load_actions_str(
            source,
            &lili_actions::ActionLoadContext::new("/", "/", Vec::new()),
        )
    }

    #[cfg(unix)]
    async fn wait_for_action_audit(state: &AppState, entries: usize) -> Vec<ActionAuditEntry> {
        for _ in 0..100 {
            let audit = state.action_audit().await;
            if audit.len() >= entries {
                return audit;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("action audit did not reach the expected length");
    }

    #[tokio::test]
    async fn app_state_serializes_concurrent_duplicate_reduction() {
        let state = AppState::default();
        let event = normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("turn_started".to_owned()),
            event_id: Some("event-1".to_owned()),
            session_id: Some("session-1".to_owned()),
            turn_id: Some("turn-1".to_owned()),
            occurred_at_ms: Some(1),
            project: None,
            summary: None,
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap();

        let (left, right) = tokio::join!(
            state.apply_session_event(event.clone()),
            state.apply_session_event(event)
        );
        assert!(matches!(
            (left, right),
            (
                ReductionOutcome::Applied { revision: 1 },
                ReductionOutcome::Duplicate
            ) | (
                ReductionOutcome::Duplicate,
                ReductionOutcome::Applied { revision: 1 }
            )
        ));
        let snapshot = state.snapshot().await;
        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.session_state.sessions.len(), 1);
    }

    #[tokio::test]
    async fn persistent_application_state_restores_session_reducer() {
        let state = AppState::default();
        let event = normalize_provider_input(ProviderInputV1 {
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
            source_discriminator: None,
        })
        .unwrap();
        state.apply_session_event(event).await;
        let placement = WindowPlacement::new("display-1", 10, 20, 2_000).unwrap();
        let persistent = state.persistent_state(Some(placement.clone())).await;
        assert_eq!(persistent.window_placement(), Some(&placement));

        let restored = AppState::with_persistent_state(PetCatalog::default(), persistent).unwrap();
        let snapshot = restored.snapshot().await;
        assert_eq!(snapshot.revision, 1);
        assert_eq!(
            snapshot.session_state.sessions[0].phase,
            SessionPhase::Completed
        );
        assert_eq!(snapshot.session_state.notifications.len(), 1);
    }

    #[tokio::test]
    async fn persisted_event_commit_restores_after_restart() {
        let root = std::env::temp_dir().join(format!(
            "lili-app-state-transition-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = ApplicationPaths::from_root(root).unwrap();
        let store = AppStateStore::for_application(paths.clone());
        let state = AppState::default();
        let event = normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("turn_completed".to_owned()),
            event_id: Some("event-transition".to_owned()),
            session_id: Some("session-transition".to_owned()),
            turn_id: Some("turn-transition".to_owned()),
            occurred_at_ms: Some(10),
            project: None,
            summary: None,
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap();

        assert!(matches!(
            state
                .apply_session_event_persisted(event, &store)
                .await
                .unwrap(),
            ReductionOutcome::Applied { revision: 1 }
        ));
        let persisted = store.load().unwrap().unwrap();
        let restored = AppState::with_persistent_state(PetCatalog::default(), persisted).unwrap();
        assert_eq!(restored.snapshot().await.session_state.revision, 1);
        std::fs::remove_dir_all(paths.root()).unwrap();
    }

    #[tokio::test]
    async fn persisted_notification_acknowledgement_survives_restart() {
        let root = std::env::temp_dir().join(format!(
            "lili-app-state-acknowledgement-{}",
            uuid::Uuid::new_v4()
        ));
        let paths = ApplicationPaths::from_root(root).unwrap();
        let store = AppStateStore::for_application(paths.clone());
        let state = AppState::default();
        let event = normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("turn_completed".to_owned()),
            event_id: Some("event-acknowledgement".to_owned()),
            session_id: Some("session-acknowledgement".to_owned()),
            turn_id: Some("turn-acknowledgement".to_owned()),
            occurred_at_ms: Some(10),
            project: None,
            summary: None,
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap();

        state
            .apply_session_event_persisted(event, &store)
            .await
            .unwrap();
        let notification_id = state.snapshot().await.session_state.notifications[0]
            .id
            .clone();
        assert!(matches!(
            state
                .acknowledge_notification_persisted(&notification_id, 11, &store)
                .await
                .unwrap(),
            ReductionOutcome::Applied { revision: 2 }
        ));

        let persisted = store.load().unwrap().unwrap();
        let restored = AppState::with_persistent_state(PetCatalog::default(), persisted).unwrap();
        assert!(
            restored
                .snapshot()
                .await
                .session_state
                .notifications
                .is_empty()
        );
        std::fs::remove_dir_all(paths.root()).unwrap();
    }

    #[tokio::test]
    async fn pet_presentation_is_derived_in_native_state() {
        let state = AppState::default();
        let event = normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("turn_completed".to_owned()),
            event_id: Some("event-presentation".to_owned()),
            session_id: Some("session-private".to_owned()),
            turn_id: Some("turn-private".to_owned()),
            occurred_at_ms: Some(10),
            project: Some(ProviderProjectInputV1 {
                label: Some("Workspace".to_owned()),
            }),
            summary: Some("Finished safely".to_owned()),
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap();
        state.apply_session_event(event).await;
        let presentation = state.pet_presentation().await;
        assert_eq!(presentation.revision, 1);
        assert_eq!(presentation.lifecycle, PetLifecycleState::Review);
        assert_eq!(presentation.unread_notification_count, 1);
        let card = &presentation.notifications[0];
        assert_eq!(card.kind, PetNotificationKind::Completion);
        assert_eq!(card.project_label.as_deref(), Some("Workspace"));
        assert_eq!(card.summary, "Finished safely");
        let notification_id = NotificationId::parse(card.activation_id.clone()).unwrap();
        let context = state.notification_context(&notification_id).await.unwrap();
        assert_eq!(context.session_id.as_str(), "session-private");
        assert_eq!(context.turn_id.as_ref().unwrap().as_str(), "turn-private");

        let next_turn = normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("turn_started".to_owned()),
            event_id: Some("event-next-turn".to_owned()),
            session_id: Some("session-private".to_owned()),
            turn_id: Some("turn-next".to_owned()),
            occurred_at_ms: Some(20),
            project: None,
            summary: None,
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap();
        state.apply_session_event(next_turn).await;
        let updated = state.pet_presentation().await;
        assert_eq!(updated.notifications[0].activation_id, card.activation_id);
        let context = state.notification_context(&notification_id).await.unwrap();
        assert_eq!(context.turn_id.as_ref().unwrap().as_str(), "turn-private");
        let serialized = serde_json::to_string(&presentation).unwrap();
        assert!(!serialized.contains("session-private"));
        assert!(!serialized.contains("turn-private"));
    }

    #[tokio::test]
    async fn accepted_interaction_keeps_the_clicked_notification_snapshot() {
        let state = AppState::default();
        let clicked = normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("turn_completed".to_owned()),
            event_id: Some("event-clicked".to_owned()),
            session_id: Some("session-clicked".to_owned()),
            turn_id: Some("turn-clicked".to_owned()),
            occurred_at_ms: Some(10),
            project: Some(ProviderProjectInputV1 {
                label: Some("Clicked workspace".to_owned()),
            }),
            summary: Some("Clicked completion".to_owned()),
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap();
        state.apply_session_event(clicked).await;
        let notification_id = state.snapshot().await.session_state.notifications[0]
            .id
            .clone();
        let context = state
            .bind_interaction(
                Uuid::nil(),
                11,
                InteractionTrigger::NotificationActivate,
                Some(&notification_id),
            )
            .await
            .unwrap();

        let newer = normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("attention_required".to_owned()),
            event_id: Some("event-newer".to_owned()),
            session_id: Some("session-newer".to_owned()),
            turn_id: Some("turn-newer".to_owned()),
            occurred_at_ms: Some(20),
            project: None,
            summary: Some("Newer attention".to_owned()),
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap();
        state.apply_session_event(newer).await;

        let notification = context.notification.as_ref().unwrap();
        assert_eq!(notification.event_id, "event-clicked");
        assert_eq!(notification.session_id, "session-clicked");
        assert_eq!(notification.turn_id.as_deref(), Some("turn-clicked"));
        assert_eq!(
            notification.project_label.as_deref(),
            Some("Clicked workspace")
        );
        assert_eq!(
            notification
                .summary
                .as_ref()
                .map(|summary| summary.text.as_str()),
            Some("Clicked completion")
        );
        let encoded = serde_json::to_vec(&context).unwrap();
        assert!(encoded.len() <= lili_actions::MAX_INTERACTION_CONTEXT_BYTES);
        assert!(
            !String::from_utf8(encoded)
                .unwrap()
                .contains("session-newer")
        );
    }

    #[tokio::test]
    async fn pet_interaction_binds_pet_state_without_session_context() {
        let state = AppState::default();
        let context = state
            .bind_interaction(Uuid::nil(), 1, InteractionTrigger::PetDoubleClick, None)
            .await
            .unwrap();
        assert_eq!(context.trigger, InteractionTrigger::PetDoubleClick);
        assert_eq!(context.pet.pet_id, "lili");
        assert_eq!(context.pet.lifecycle, PetLifecycleSnapshotV1::Idle);
        assert!(context.notification.is_none());
    }

    #[tokio::test]
    async fn action_failure_feedback_does_not_mutate_session_or_notification_state() {
        let state = AppState::default();
        let event = normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("turn_completed".to_owned()),
            event_id: Some("event-action-feedback".to_owned()),
            session_id: Some("session-action-feedback".to_owned()),
            turn_id: Some("turn-action-feedback".to_owned()),
            occurred_at_ms: Some(10),
            project: None,
            summary: None,
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap();
        state.apply_session_event(event).await;
        let before = state.snapshot().await;
        let result = ActionExecutionResult {
            action_id: "open-session".to_owned(),
            interaction_id: Uuid::nil(),
            trigger: InteractionTrigger::NotificationActivate,
            event_id: Some("event-action-feedback".to_owned()),
            started_at_ms: 11,
            finished_at_ms: 12,
            outcome: ActionExecutionOutcome::SpawnFailed,
            exit_code: None,
            stdout: lili_actions::CapturedOutput::default(),
            stderr: lili_actions::CapturedOutput::default(),
        };
        assert!(state.publish_action_result(&result).await);

        let after = state.snapshot().await;
        assert!(after.revision > before.revision);
        assert_eq!(after.session_state.revision, before.session_state.revision);
        assert_eq!(
            after.session_state.sessions[0].phase,
            before.session_state.sessions[0].phase
        );
        assert_eq!(
            after.session_state.notifications[0].state,
            NotificationState::Unread
        );
        let presentation = state.pet_presentation().await;
        let feedback = presentation.action_feedback.unwrap();
        assert_eq!(feedback.action_id, "open-session");
        assert_eq!(feedback.kind, PetActionFeedbackKind::Failure);
        assert_eq!(feedback.message, "Action could not start");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn interaction_dispatches_each_matching_action_at_most_once() {
        let state = AppState::default();
        let loaded = test_actions(
            r#"
version = 1

[[action]]
id = "wave"
trigger = "pet_click"
command = ["/bin/cat"]
debounce_ms = 0

[[action]]
id = "jump"
trigger = "pet_double_click"
command = ["/bin/cat"]
debounce_ms = 0
"#,
        );
        assert!(state.configure_actions(loaded, 1).await);
        let context = state
            .bind_interaction(Uuid::new_v4(), 1, InteractionTrigger::PetClick, None)
            .await
            .unwrap();
        let (left, right) = tokio::join!(
            state.dispatch_interaction(context.clone()),
            state.dispatch_interaction(context),
        );
        assert!(left.accepted ^ right.accepted);
        assert_eq!(left.action_count + right.action_count, 1);
        let audit = wait_for_action_audit(&state, 1).await;
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].action_id, "wave");
        assert_eq!(audit[0].outcome, ActionExecutionOutcome::Succeeded);

        let double_click = state
            .bind_interaction(Uuid::new_v4(), 2, InteractionTrigger::PetDoubleClick, None)
            .await
            .unwrap();
        let dispatch = state.dispatch_interaction(double_click).await;
        assert!(dispatch.accepted);
        assert_eq!(dispatch.action_count, 1);
        let audit = wait_for_action_audit(&state, 2).await;
        assert_eq!(audit[1].action_id, "jump");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn notification_dispatch_keeps_bound_event_during_concurrent_updates() {
        let state = AppState::default();
        let loaded = test_actions(
            r#"
version = 1

[[action]]
id = "open-completion"
trigger = "notification_activate"
command = ["/bin/sh", "-c", "exit 0"]
debounce_ms = 0

[action.filters]
notification_kinds = ["completion"]
providers = ["codex"]
"#,
        );
        assert!(state.configure_actions(loaded, 1).await);
        let clicked = normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("turn_completed".to_owned()),
            event_id: Some("event-clicked-dispatch".to_owned()),
            session_id: Some("session-clicked-dispatch".to_owned()),
            turn_id: Some("turn-clicked-dispatch".to_owned()),
            occurred_at_ms: Some(10),
            project: None,
            summary: None,
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap();
        state.apply_session_event(clicked).await;
        let notification_id = state.snapshot().await.session_state.notifications[0]
            .id
            .clone();
        let context = state
            .bind_interaction(
                Uuid::new_v4(),
                11,
                InteractionTrigger::NotificationActivate,
                Some(&notification_id),
            )
            .await
            .unwrap();
        let newer = normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("attention_required".to_owned()),
            event_id: Some("event-newer-dispatch".to_owned()),
            session_id: Some("session-newer-dispatch".to_owned()),
            turn_id: Some("turn-newer-dispatch".to_owned()),
            occurred_at_ms: Some(20),
            project: None,
            summary: None,
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap();
        let (dispatch, _) = tokio::join!(
            state.dispatch_interaction(context),
            state.apply_session_event(newer),
        );
        assert!(dispatch.accepted);
        assert_eq!(dispatch.action_count, 1);
        let audit = wait_for_action_audit(&state, 1).await;
        assert_eq!(audit[0].event_id.as_deref(), Some("event-clicked-dispatch"));
        assert_eq!(state.pet_presentation().await.unread_notification_count, 2);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rapid_click_debounce_emits_only_one_ui_result() {
        let state = AppState::default();
        let loaded = test_actions(
            r#"
version = 1

[[action]]
id = "wave-once"
trigger = "pet_click"
command = ["/bin/sh", "-c", "exit 0"]
debounce_ms = 1000
"#,
        );
        assert!(state.configure_actions(loaded, 1).await);
        let mut presentations = state.subscribe_pet_presentation();
        let first = state
            .bind_interaction(Uuid::new_v4(), 1, InteractionTrigger::PetClick, None)
            .await
            .unwrap();
        assert!(state.dispatch_interaction(first).await.accepted);
        wait_for_action_audit(&state, 1).await;
        presentations.changed().await.unwrap();
        let first_feedback = presentations
            .borrow_and_update()
            .action_feedback
            .clone()
            .unwrap();
        assert_eq!(first_feedback.kind, PetActionFeedbackKind::Success);

        let second = state
            .bind_interaction(Uuid::new_v4(), 2, InteractionTrigger::PetClick, None)
            .await
            .unwrap();
        assert!(state.dispatch_interaction(second).await.accepted);
        let audit = wait_for_action_audit(&state, 2).await;
        assert_eq!(audit[0].outcome, ActionExecutionOutcome::Succeeded);
        assert_eq!(audit[1].outcome, ActionExecutionOutcome::Debounced);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(!presentations.has_changed().unwrap());
    }

    #[tokio::test]
    async fn presentation_subscriber_observes_only_monotonic_applied_revisions() {
        let state = AppState::default();
        let mut presentations = state.subscribe_pet_presentation();
        let input = ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("turn_started".to_owned()),
            event_id: Some("event-stream".to_owned()),
            session_id: Some("session-stream".to_owned()),
            turn_id: Some("turn-stream".to_owned()),
            occurred_at_ms: Some(10),
            project: None,
            summary: None,
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        };
        let event = normalize_provider_input(input).unwrap();
        assert!(matches!(
            state.apply_session_event(event.clone()).await,
            ReductionOutcome::Applied { revision: 1 }
        ));
        presentations.changed().await.unwrap();
        assert_eq!(presentations.borrow_and_update().revision, 1);
        assert_eq!(
            state.apply_session_event(event).await,
            ReductionOutcome::Duplicate
        );
        assert!(!presentations.has_changed().unwrap());
    }

    #[tokio::test]
    async fn activity_reminder_expires_after_an_active_event() {
        let state = AppState::default();
        let mut presentations = state.subscribe_pet_presentation();
        let event = normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("turn_started".to_owned()),
            event_id: Some("event-activity-reminder".to_owned()),
            session_id: Some("session-activity-reminder".to_owned()),
            turn_id: Some("turn-activity-reminder".to_owned()),
            occurred_at_ms: Some(0),
            project: None,
            summary: None,
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap();

        state.apply_session_event(event).await;
        presentations.changed().await.unwrap();
        assert_eq!(
            presentations.borrow_and_update().lifecycle,
            PetLifecycleState::ActivityReminder
        );
        tokio::time::timeout(Duration::from_secs(6), presentations.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            presentations.borrow_and_update().lifecycle,
            PetLifecycleState::Idle
        );
    }

    #[tokio::test]
    async fn pet_replacement_publishes_a_monotonic_ui_revision() {
        let state = AppState::default();
        let mut presentations = state.subscribe_pet_presentation();
        assert!(state.settings().await.always_on_top);
        assert!(
            state
                .available_pets()
                .await
                .iter()
                .any(|pet| pet.id.as_str() == lili_pet::DEFAULT_PET_ID)
        );

        state.replace_pet_catalog(PetCatalog::default()).await;
        presentations.changed().await.unwrap();
        assert_eq!(presentations.borrow_and_update().revision, 1);
        assert_eq!(state.snapshot().await.revision, 1);
        assert_eq!(state.snapshot().await.session_state.revision, 0);
    }

    #[tokio::test]
    async fn reduced_motion_setting_publishes_without_session_mutation() {
        let state = AppState::default();
        let mut presentations = state.subscribe_pet_presentation();
        state
            .replace_settings(UserSettings {
                always_on_top: true,
                reduced_motion: true,
            })
            .await;
        presentations.changed().await.unwrap();
        assert!(presentations.borrow_and_update().reduced_motion);
        assert_eq!(state.snapshot().await.revision, 1);
        assert_eq!(state.snapshot().await.session_state.revision, 0);
    }
}
