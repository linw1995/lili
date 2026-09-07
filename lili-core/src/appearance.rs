use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::PetId;

pub const MAX_APPEARANCE_PETS: usize = 64;
pub const MAX_APPEARANCE_DISPLAY_NAME_BYTES: usize = 128;
pub const MAX_APPEARANCE_ASSET_ID_BYTES: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AppearancePetView {
    pub id: PetId,
    pub display_name: String,
    pub asset_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AppearanceView {
    pub pets: Vec<AppearancePetView>,
    pub selected_pet_id: Option<PetId>,
}

impl AppearanceView {
    pub fn validate(&self) -> Result<(), AppearanceContractError> {
        if self.pets.len() > MAX_APPEARANCE_PETS {
            return Err(AppearanceContractError::TooManyPets);
        }

        let mut pet_ids = HashSet::with_capacity(self.pets.len());
        let mut asset_ids = HashSet::with_capacity(self.pets.len());
        for pet in &self.pets {
            if PetId::parse(pet.id.as_str()).is_none() {
                return Err(AppearanceContractError::InvalidPetId);
            }
            if pet.display_name.is_empty()
                || pet.display_name.len() > MAX_APPEARANCE_DISPLAY_NAME_BYTES
                || pet.display_name.chars().any(char::is_control)
            {
                return Err(AppearanceContractError::InvalidDisplayName);
            }
            if !valid_asset_id(&pet.asset_id) {
                return Err(AppearanceContractError::InvalidAssetId);
            }
            if !pet_ids.insert(&pet.id) {
                return Err(AppearanceContractError::DuplicatePetId);
            }
            if !asset_ids.insert(&pet.asset_id) {
                return Err(AppearanceContractError::DuplicateAssetId);
            }
        }

        if self
            .selected_pet_id
            .as_ref()
            .is_some_and(|selected| !pet_ids.contains(selected))
        {
            return Err(AppearanceContractError::SelectedPetUnavailable);
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AppearanceSelectionRequest {
    pub pet_id: PetId,
}

impl AppearanceSelectionRequest {
    pub fn validate(&self) -> Result<(), AppearanceContractError> {
        PetId::parse(self.pet_id.as_str())
            .map(|_| ())
            .ok_or(AppearanceContractError::InvalidPetId)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppearanceErrorCode {
    InvalidPetId,
    PetUnavailable,
    AssetUnavailable,
    PersistenceUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AppearanceErrorResponse {
    pub code: AppearanceErrorCode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppearanceContractError {
    TooManyPets,
    InvalidPetId,
    InvalidDisplayName,
    InvalidAssetId,
    DuplicatePetId,
    DuplicateAssetId,
    SelectedPetUnavailable,
}

fn valid_asset_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_APPEARANCE_ASSET_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn pet(id: &str, display_name: &str, asset_id: &str) -> AppearancePetView {
        AppearancePetView {
            id: PetId::parse(id).unwrap(),
            display_name: display_name.to_owned(),
            asset_id: asset_id.to_owned(),
        }
    }

    #[test]
    fn appearance_view_round_trips_with_bounded_fields() {
        let view = AppearanceView {
            pets: vec![pet("lili", "Lili", "asset-1")],
            selected_pet_id: Some(PetId::parse("lili").unwrap()),
        };

        view.validate().unwrap();
        let encoded = serde_json::to_value(&view).unwrap();
        assert_eq!(encoded["pets"][0]["displayName"], "Lili");
        assert_eq!(encoded["pets"][0]["assetId"], "asset-1");
        assert_eq!(encoded["selectedPetId"], "lili");
        assert_eq!(
            serde_json::from_value::<AppearanceView>(encoded).unwrap(),
            view
        );
    }

    #[test]
    fn selection_request_rejects_unknown_fields() {
        let request = serde_json::from_value::<AppearanceSelectionRequest>(json!({
            "petId": "lili",
            "path": "/tmp/pet"
        }));

        assert!(request.is_err());
    }

    #[test]
    fn view_rejects_paths_duplicates_and_unavailable_selection() {
        let invalid_asset = AppearanceView {
            pets: vec![pet("lili", "Lili", "../spritesheet")],
            selected_pet_id: Some(PetId::parse("lili").unwrap()),
        };
        assert_eq!(
            invalid_asset.validate(),
            Err(AppearanceContractError::InvalidAssetId)
        );

        let duplicate = AppearanceView {
            pets: vec![
                pet("lili", "Lili", "asset-1"),
                pet("lili", "Lili 2", "asset-2"),
            ],
            selected_pet_id: Some(PetId::parse("lili").unwrap()),
        };
        assert_eq!(
            duplicate.validate(),
            Err(AppearanceContractError::DuplicatePetId)
        );

        let unavailable = AppearanceView {
            pets: vec![pet("lili", "Lili", "asset-1")],
            selected_pet_id: Some(PetId::parse("missing").unwrap()),
        };
        assert_eq!(
            unavailable.validate(),
            Err(AppearanceContractError::SelectedPetUnavailable)
        );
    }

    #[test]
    fn error_response_has_a_fixed_code_only_shape() {
        let response = AppearanceErrorResponse {
            code: AppearanceErrorCode::PetUnavailable,
        };
        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({"code": "pet_unavailable"})
        );
    }
}
