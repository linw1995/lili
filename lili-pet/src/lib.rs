mod discovery;
mod selection;
mod validation;

use std::time::Duration;

use lili_core::PetId;
use serde::{Deserialize, Serialize};

pub use discovery::{
    DEFAULT_PET_ID, DiscoveredPackage, DiscoveryIssue, DiscoveryReport, PackageOrigin,
    default_pet_path, discover_pet_packages, resolve_codex_home,
};
pub use selection::{
    AvailablePet, CatalogDiagnostic, PetAssetSource, PetCatalog, SelectionError,
    persist_selected_pet, selection_path,
};
pub use validation::{
    AtlasFormat, AtlasMetadata, AtlasValidationError, ValidatedPetPackage,
    validate_discovered_package,
};

pub const SPRITE_VERSION_NUMBER: u8 = 2;
pub const ATLAS_COLUMNS: u8 = 8;
pub const ATLAS_ROWS: u8 = 11;
pub const CELL_WIDTH: u32 = 192;
pub const CELL_HEIGHT: u32 = 208;
pub const ATLAS_WIDTH: u32 = CELL_WIDTH * ATLAS_COLUMNS as u32;
pub const ATLAS_HEIGHT: u32 = CELL_HEIGHT * ATLAS_ROWS as u32;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetManifest {
    id: String,
    display_name: String,
    description: String,
    sprite_version_number: u8,
    spritesheet_path: String,
}

impl PetManifest {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn sprite_version_number(&self) -> u8 {
        self.sprite_version_number
    }

    pub fn spritesheet_path(&self) -> &str {
        &self.spritesheet_path
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PetDefinition {
    id: PetId,
    display_name: String,
    description: String,
}

impl PetDefinition {
    pub fn id(&self) -> &PetId {
        &self.id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn description(&self) -> &str {
        &self.description
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PetSummary {
    pub id: PetId,
    pub display_name: String,
}

impl From<&PetDefinition> for PetSummary {
    fn from(definition: &PetDefinition) -> Self {
        Self {
            id: definition.id.clone(),
            display_name: definition.display_name.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AnimationState {
    Idle,
    RunningRight,
    RunningLeft,
    Waving,
    Jumping,
    Failed,
    Waiting,
    Running,
    Review,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnimationSpec {
    state: AnimationState,
    row: u8,
    frame_durations_ms: &'static [u16],
}

impl AnimationSpec {
    pub const fn state(self) -> AnimationState {
        self.state
    }

    pub const fn row(self) -> u8 {
        self.row
    }

    pub const fn frame_count(self) -> usize {
        self.frame_durations_ms.len()
    }

    pub fn frames(self) -> impl ExactSizeIterator<Item = FrameDescriptor> {
        self.frame_durations_ms
            .iter()
            .enumerate()
            .map(move |(column, duration_ms)| FrameDescriptor {
                row: self.row,
                column: column as u8,
                duration: Duration::from_millis(u64::from(*duration_ms)),
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameDescriptor {
    row: u8,
    column: u8,
    duration: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AtlasCell {
    row: u8,
    column: u8,
}

impl AtlasCell {
    pub const fn row(self) -> u8 {
        self.row
    }

    pub const fn column(self) -> u8 {
        self.column
    }
}

pub const NEUTRAL_LOOK_CELL: AtlasCell = AtlasCell { row: 0, column: 6 };

impl FrameDescriptor {
    pub const fn row(self) -> u8 {
        self.row
    }

    pub const fn column(self) -> u8 {
        self.column
    }

    pub const fn duration(self) -> Duration {
        self.duration
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LookFrame {
    index: u8,
    degrees_eighths: u16,
    row: u8,
    column: u8,
}

impl LookFrame {
    pub const fn index(self) -> u8 {
        self.index
    }

    pub fn degrees(self) -> f32 {
        f32::from(self.degrees_eighths) / 8.0
    }

    pub const fn row(self) -> u8 {
        self.row
    }

    pub const fn column(self) -> u8 {
        self.column
    }
}

const IDLE_DURATIONS: &[u16] = &[280, 110, 110, 140, 140, 320];
const RUNNING_RIGHT_DURATIONS: &[u16] = &[120, 120, 120, 120, 120, 120, 120, 220];
const RUNNING_LEFT_DURATIONS: &[u16] = RUNNING_RIGHT_DURATIONS;
const WAVING_DURATIONS: &[u16] = &[140, 140, 140, 280];
const JUMPING_DURATIONS: &[u16] = &[140, 140, 140, 140, 280];
const FAILED_DURATIONS: &[u16] = &[140, 140, 140, 140, 140, 140, 140, 240];
const WAITING_DURATIONS: &[u16] = &[150, 150, 150, 150, 150, 260];
const RUNNING_DURATIONS: &[u16] = &[120, 120, 120, 120, 120, 220];
const REVIEW_DURATIONS: &[u16] = &[150, 150, 150, 150, 150, 280];

pub const STANDARD_ANIMATIONS: [AnimationSpec; 9] = [
    AnimationSpec {
        state: AnimationState::Idle,
        row: 0,
        frame_durations_ms: IDLE_DURATIONS,
    },
    AnimationSpec {
        state: AnimationState::RunningRight,
        row: 1,
        frame_durations_ms: RUNNING_RIGHT_DURATIONS,
    },
    AnimationSpec {
        state: AnimationState::RunningLeft,
        row: 2,
        frame_durations_ms: RUNNING_LEFT_DURATIONS,
    },
    AnimationSpec {
        state: AnimationState::Waving,
        row: 3,
        frame_durations_ms: WAVING_DURATIONS,
    },
    AnimationSpec {
        state: AnimationState::Jumping,
        row: 4,
        frame_durations_ms: JUMPING_DURATIONS,
    },
    AnimationSpec {
        state: AnimationState::Failed,
        row: 5,
        frame_durations_ms: FAILED_DURATIONS,
    },
    AnimationSpec {
        state: AnimationState::Waiting,
        row: 6,
        frame_durations_ms: WAITING_DURATIONS,
    },
    AnimationSpec {
        state: AnimationState::Running,
        row: 7,
        frame_durations_ms: RUNNING_DURATIONS,
    },
    AnimationSpec {
        state: AnimationState::Review,
        row: 8,
        frame_durations_ms: REVIEW_DURATIONS,
    },
];

pub const LOOK_DIRECTIONS: [LookFrame; 16] = [
    look_frame(0),
    look_frame(1),
    look_frame(2),
    look_frame(3),
    look_frame(4),
    look_frame(5),
    look_frame(6),
    look_frame(7),
    look_frame(8),
    look_frame(9),
    look_frame(10),
    look_frame(11),
    look_frame(12),
    look_frame(13),
    look_frame(14),
    look_frame(15),
];

const fn look_frame(index: u8) -> LookFrame {
    LookFrame {
        index,
        degrees_eighths: index as u16 * 180,
        row: 9 + index / ATLAS_COLUMNS,
        column: index % ATLAS_COLUMNS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_geometry_is_exact() {
        assert_eq!((ATLAS_WIDTH, ATLAS_HEIGHT), (1536, 2288));
        assert_eq!((ATLAS_COLUMNS, ATLAS_ROWS), (8, 11));
        assert_eq!((CELL_WIDTH, CELL_HEIGHT), (192, 208));
        assert_eq!(
            (NEUTRAL_LOOK_CELL.row(), NEUTRAL_LOOK_CELL.column()),
            (0, 6)
        );
    }

    #[test]
    fn standard_animation_contract_is_exact() {
        let expected = [
            (AnimationState::Idle, 0, vec![280, 110, 110, 140, 140, 320]),
            (
                AnimationState::RunningRight,
                1,
                vec![120, 120, 120, 120, 120, 120, 120, 220],
            ),
            (
                AnimationState::RunningLeft,
                2,
                vec![120, 120, 120, 120, 120, 120, 120, 220],
            ),
            (AnimationState::Waving, 3, vec![140, 140, 140, 280]),
            (AnimationState::Jumping, 4, vec![140, 140, 140, 140, 280]),
            (
                AnimationState::Failed,
                5,
                vec![140, 140, 140, 140, 140, 140, 140, 240],
            ),
            (
                AnimationState::Waiting,
                6,
                vec![150, 150, 150, 150, 150, 260],
            ),
            (
                AnimationState::Running,
                7,
                vec![120, 120, 120, 120, 120, 220],
            ),
            (
                AnimationState::Review,
                8,
                vec![150, 150, 150, 150, 150, 280],
            ),
        ];

        for (spec, (state, row, durations)) in STANDARD_ANIMATIONS.into_iter().zip(expected) {
            assert_eq!(spec.state(), state);
            assert_eq!(spec.row(), row);
            assert_eq!(spec.frame_count(), durations.len());
            assert_eq!(
                spec.frames()
                    .map(|frame| frame.duration().as_millis())
                    .collect::<Vec<_>>(),
                durations
            );
        }
    }

    #[test]
    fn look_directions_are_clockwise_from_up() {
        for (index, frame) in LOOK_DIRECTIONS.into_iter().enumerate() {
            assert_eq!(frame.index(), index as u8);
            assert_eq!(frame.degrees(), index as f32 * 22.5);
            assert_eq!(frame.row(), 9 + (index / 8) as u8);
            assert_eq!(frame.column(), (index % 8) as u8);
        }
    }

    #[test]
    fn manifest_uses_v2_external_field_names() {
        let manifest: PetManifest = serde_json::from_str(
            r#"{
                "id": "lili",
                "displayName": "Lili",
                "description": "A desktop pet.",
                "spriteVersionNumber": 2,
                "spritesheetPath": "spritesheet.webp"
            }"#,
        )
        .unwrap();
        assert_eq!(manifest.id(), "lili");
        assert_eq!(manifest.sprite_version_number(), SPRITE_VERSION_NUMBER);
        assert_eq!(manifest.spritesheet_path(), "spritesheet.webp");
    }
}
