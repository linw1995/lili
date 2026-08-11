use std::sync::Arc;

use lili_actions::ActionSummary;
use lili_pet::{PetCatalog, PetSummary};
use lili_session::SessionSummary;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ViewSnapshot {
    pub revision: u64,
    pub selected_pet: Option<PetSummary>,
    pub sessions: Vec<SessionSummary>,
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
    pet_catalog: Arc<PetCatalog>,
}

impl AppState {
    pub fn with_pet_catalog(pet_catalog: PetCatalog) -> Self {
        let selected_pet = Some(PetSummary::from(pet_catalog.active().definition()));
        Self {
            snapshot: Arc::new(RwLock::new(ViewSnapshot {
                selected_pet,
                ..ViewSnapshot::default()
            })),
            settings: Arc::new(RwLock::new(UserSettings::default())),
            pet_catalog: Arc::new(pet_catalog),
        }
    }

    pub fn pet_catalog(&self) -> &PetCatalog {
        &self.pet_catalog
    }

    pub async fn snapshot(&self) -> ViewSnapshot {
        self.snapshot.read().await.clone()
    }

    pub async fn settings(&self) -> UserSettings {
        self.settings.read().await.clone()
    }

    pub async fn replace_settings(&self, settings: UserSettings) -> UserSettings {
        *self.settings.write().await = settings.clone();
        settings
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::with_pet_catalog(PetCatalog::default())
    }
}
