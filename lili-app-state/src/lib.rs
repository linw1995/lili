mod ingestion;
mod persistence;

use std::sync::Arc;

use lili_actions::ActionSummary;
use lili_core::{
    PetLifecycleState, PetNotificationKind, PetNotificationPresentation, PetPresentationState,
};
use lili_pet::{AtlasFormat, PetCatalog, PetSummary};
use lili_session::{
    NormalizedSessionEvent, Notification, NotificationId, NotificationKind, NotificationState,
    PresentationState, ReductionOutcome, SessionReducer, SessionViewSnapshot,
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
    presentation_sender: Arc<watch::Sender<PetPresentationState>>,
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
        let (presentation_sender, _) = watch::channel(presentation_from_view(&initial_snapshot));
        Self {
            snapshot: Arc::new(RwLock::new(initial_snapshot)),
            settings: Arc::new(RwLock::new(UserSettings::default())),
            session_reducer: Arc::new(Mutex::new(reducer)),
            pet_catalog: Arc::new(RwLock::new(pet_catalog)),
            pet_asset: Arc::new(RwLock::new(pet_asset)),
            ingestion_diagnostics: Arc::new(RwLock::new(IngestionDiagnostics::default())),
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
        presentation_from_view(&snapshot)
    }

    pub fn subscribe_pet_presentation(&self) -> watch::Receiver<PetPresentationState> {
        self.presentation_sender.subscribe()
    }

    pub async fn settings(&self) -> UserSettings {
        self.settings.read().await.clone()
    }

    pub async fn replace_settings(&self, settings: UserSettings) -> UserSettings {
        *self.settings.write().await = settings.clone();
        settings
    }

    pub async fn apply_session_event(&self, event: NormalizedSessionEvent) -> ReductionOutcome {
        let outcome = self.session_reducer.lock().await.reduce(event);
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
        let presentation = {
            let mut snapshot = self.snapshot.write().await;
            snapshot.session_state = session_state;
            snapshot.revision = snapshot.revision.saturating_add(1);
            presentation_from_view(&snapshot)
        };
        self.presentation_sender.send_replace(presentation);
    }
}

fn presentation_from_view(snapshot: &ViewSnapshot) -> PetPresentationState {
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
            PresentationState::Running => PetLifecycleState::Running,
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

    use super::*;

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
}
