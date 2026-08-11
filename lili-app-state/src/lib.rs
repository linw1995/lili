use std::sync::Arc;

use lili_actions::ActionSummary;
use lili_pet::{AtlasFormat, PetCatalog, PetSummary};
use lili_session::{
    NormalizedSessionEvent, NotificationId, ReductionOutcome, SessionReducer, SessionViewSnapshot,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ViewSnapshot {
    pub revision: u64,
    pub selected_pet: Option<PetSummary>,
    pub pet_asset_id: Option<String>,
    pub session_state: SessionViewSnapshot,
    pub actions: Vec<ActionSummary>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct UserSettings {
    pub always_on_top: bool,
    pub reduced_motion: bool,
}

#[derive(Clone)]
pub struct AppState {
    snapshot: Arc<RwLock<ViewSnapshot>>,
    settings: Arc<RwLock<UserSettings>>,
    session_reducer: Arc<Mutex<SessionReducer>>,
    pet_catalog: Arc<PetCatalog>,
    pet_asset: Arc<ApprovedPetAsset>,
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
        let (pet_catalog, pet_asset) = load_active_asset(pet_catalog);
        let selected_pet = Some(PetSummary::from(pet_catalog.active().definition()));
        let pet_asset_id = Some(pet_asset.id().to_owned());
        Self {
            snapshot: Arc::new(RwLock::new(ViewSnapshot {
                selected_pet,
                pet_asset_id,
                ..ViewSnapshot::default()
            })),
            settings: Arc::new(RwLock::new(UserSettings::default())),
            session_reducer: Arc::new(Mutex::new(SessionReducer::default())),
            pet_catalog: Arc::new(pet_catalog),
            pet_asset: Arc::new(pet_asset),
        }
    }

    pub fn pet_catalog(&self) -> &PetCatalog {
        &self.pet_catalog
    }

    pub fn approved_pet_asset(&self, asset_id: &str) -> Option<Arc<ApprovedPetAsset>> {
        (self.pet_asset.id() == asset_id).then(|| Arc::clone(&self.pet_asset))
    }

    pub async fn snapshot(&self) -> ViewSnapshot {
        let mut snapshot = self.snapshot.read().await.clone();
        snapshot.session_state = self.session_reducer.lock().await.snapshot();
        snapshot.revision = snapshot.session_state.revision;
        snapshot
    }

    pub async fn settings(&self) -> UserSettings {
        self.settings.read().await.clone()
    }

    pub async fn replace_settings(&self, settings: UserSettings) -> UserSettings {
        *self.settings.write().await = settings.clone();
        settings
    }

    pub async fn apply_session_event(&self, event: NormalizedSessionEvent) -> ReductionOutcome {
        self.session_reducer.lock().await.reduce(event)
    }

    pub async fn acknowledge_notification(&self, id: &NotificationId) -> ReductionOutcome {
        self.session_reducer
            .lock()
            .await
            .acknowledge_notification(id)
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
    use lili_session::{ProviderCapabilitiesInputV1, ProviderInputV1, normalize_provider_input};

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
}
