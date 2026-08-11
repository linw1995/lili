use lili_core::PetId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PetSummary {
    pub id: PetId,
    pub display_name: String,
}
