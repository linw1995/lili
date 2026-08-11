use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionTrigger {
    PetClick,
    PetDoubleClick,
    NotificationActivate,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionSummary {
    pub id: String,
    pub trigger: InteractionTrigger,
    pub enabled: bool,
}
