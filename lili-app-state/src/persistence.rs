use std::path::PathBuf;

use lili_core::PetId;
use lili_session::{ReducerRestoreError, SessionReducer, SessionReducerState};
use lili_storage::{
    ApplicationPaths, DatabaseError, JsonDocument, open,
    repository::{load_app_state, update_app_state},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const PERSISTENCE_VERSION: u16 = 1;
const STORAGE_SCHEMA_VERSION: i32 = 3;
const MAX_DISPLAY_ID_CHARS: usize = 256;
const MAX_LOGICAL_COORDINATE: i32 = 1_000_000;
const MIN_SCALE_MILLI: u32 = 250;
const MAX_SCALE_MILLI: u32 = 8_000;
pub const DEFAULT_VISIBLE_WINDOW_MARGIN: u32 = 48;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayWorkArea {
    id: String,
    physical_x: i32,
    physical_y: i32,
    physical_width: u32,
    physical_height: u32,
    scale_milli: u32,
    primary: bool,
}

impl DisplayWorkArea {
    pub fn new(
        id: impl Into<String>,
        physical_x: i32,
        physical_y: i32,
        physical_width: u32,
        physical_height: u32,
        scale_milli: u32,
        primary: bool,
    ) -> Result<Self, PersistenceError> {
        let display = Self {
            id: id.into(),
            physical_x,
            physical_y,
            physical_width,
            physical_height,
            scale_milli,
            primary,
        };
        if display.id.is_empty()
            || display.id.chars().count() > MAX_DISPLAY_ID_CHARS
            || display.id.chars().any(char::is_control)
            || display.physical_width == 0
            || display.physical_height == 0
            || !(MIN_SCALE_MILLI..=MAX_SCALE_MILLI).contains(&display.scale_milli)
        {
            return Err(PersistenceError::InvalidDisplayWorkArea);
        }
        Ok(display)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub const fn scale_milli(&self) -> u32 {
        self.scale_milli
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedWindowPlacement {
    placement: WindowPlacement,
    physical_x: i32,
    physical_y: i32,
}

impl ResolvedWindowPlacement {
    pub const fn placement(&self) -> &WindowPlacement {
        &self.placement
    }

    pub const fn physical_x(&self) -> i32 {
        self.physical_x
    }

    pub const fn physical_y(&self) -> i32 {
        self.physical_y
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowPlacement {
    display_id: String,
    logical_x: i32,
    logical_y: i32,
    scale_milli: u32,
}

impl WindowPlacement {
    pub fn new(
        display_id: impl Into<String>,
        logical_x: i32,
        logical_y: i32,
        scale_milli: u32,
    ) -> Result<Self, PersistenceError> {
        let placement = Self {
            display_id: display_id.into(),
            logical_x,
            logical_y,
            scale_milli,
        };
        placement.validate()?;
        Ok(placement)
    }

    pub fn display_id(&self) -> &str {
        &self.display_id
    }

    pub const fn logical_x(&self) -> i32 {
        self.logical_x
    }

    pub const fn logical_y(&self) -> i32 {
        self.logical_y
    }

    pub const fn scale_milli(&self) -> u32 {
        self.scale_milli
    }

    fn validate(&self) -> Result<(), PersistenceError> {
        if self.display_id.is_empty()
            || self.display_id.chars().count() > MAX_DISPLAY_ID_CHARS
            || self.display_id.chars().any(char::is_control)
            || self.logical_x.unsigned_abs() > MAX_LOGICAL_COORDINATE as u32
            || self.logical_y.unsigned_abs() > MAX_LOGICAL_COORDINATE as u32
            || !(MIN_SCALE_MILLI..=MAX_SCALE_MILLI).contains(&self.scale_milli)
        {
            return Err(PersistenceError::InvalidWindowPlacement);
        }
        Ok(())
    }
}

pub fn resolve_window_placement(
    saved: &WindowPlacement,
    displays: &[DisplayWorkArea],
    physical_window_width: u32,
    physical_window_height: u32,
    visible_margin: u32,
) -> Option<ResolvedWindowPlacement> {
    let display = displays
        .iter()
        .find(|display| display.id == saved.display_id)
        .or_else(|| displays.iter().find(|display| display.primary))
        .or_else(|| displays.first())?;
    let scale = f64::from(display.scale_milli) / 1_000.0;
    let requested_x = display.physical_x + (f64::from(saved.logical_x) * scale).round() as i32;
    let requested_y = display.physical_y + (f64::from(saved.logical_y) * scale).round() as i32;
    let margin_x = visible_margin.min(physical_window_width.saturating_div(2));
    let margin_y = visible_margin.min(physical_window_height.saturating_div(2));
    let min_x =
        i64::from(display.physical_x) - i64::from(physical_window_width) + i64::from(margin_x);
    let min_y =
        i64::from(display.physical_y) - i64::from(physical_window_height) + i64::from(margin_y);
    let max_x =
        i64::from(display.physical_x) + i64::from(display.physical_width) - i64::from(margin_x);
    let max_y =
        i64::from(display.physical_y) + i64::from(display.physical_height) - i64::from(margin_y);
    let physical_x = i64::from(requested_x).clamp(min_x, max_x) as i32;
    let physical_y = i64::from(requested_y).clamp(min_y, max_y) as i32;
    let logical_x = (f64::from(physical_x - display.physical_x) / scale).round() as i32;
    let logical_y = (f64::from(physical_y - display.physical_y) / scale).round() as i32;
    let placement = WindowPlacement::new(
        display.id.clone(),
        logical_x,
        logical_y,
        display.scale_milli,
    )
    .ok()?;
    Some(ResolvedWindowPlacement {
        placement,
        physical_x,
        physical_y,
    })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistentApplicationState {
    version: u16,
    selected_pet_id: Option<PetId>,
    window_placement: Option<WindowPlacement>,
    reducer: SessionReducerState,
}

impl PersistentApplicationState {
    pub fn new(
        selected_pet_id: Option<PetId>,
        window_placement: Option<WindowPlacement>,
        reducer: SessionReducerState,
    ) -> Self {
        Self {
            version: PERSISTENCE_VERSION,
            selected_pet_id,
            window_placement,
            reducer,
        }
    }

    pub fn selected_pet_id(&self) -> Option<&PetId> {
        self.selected_pet_id.as_ref()
    }

    pub const fn window_placement(&self) -> Option<&WindowPlacement> {
        self.window_placement.as_ref()
    }

    pub(crate) fn into_reducer_state(self) -> SessionReducerState {
        self.reducer
    }

    fn validate(&self) -> Result<(), PersistenceError> {
        if self.version != PERSISTENCE_VERSION {
            return Err(PersistenceError::UnsupportedVersion(self.version));
        }
        if let Some(selected) = &self.selected_pet_id
            && PetId::parse(selected.as_str()).is_none()
        {
            return Err(PersistenceError::InvalidSelectedPet);
        }
        if let Some(placement) = &self.window_placement {
            placement.validate()?;
        }
        SessionReducer::from_persistent_state(self.reducer.clone())?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppStateStore {
    paths: ApplicationPaths,
}

impl AppStateStore {
    pub fn for_application(paths: ApplicationPaths) -> Self {
        Self { paths }
    }

    pub fn database_path(&self) -> PathBuf {
        self.paths.database_path()
    }

    pub fn load(&self) -> Result<Option<PersistentApplicationState>, PersistenceError> {
        let mut database = open(&self.paths)?;
        let row = load_app_state(database.connection())?;
        let Some(reducer_json) = row.reducer_json else {
            return Ok(None);
        };
        let reducer = serde_json::from_str::<SessionReducerState>(reducer_json.as_str())?;
        let selected_pet_id = row
            .selected_pet_id
            .map(|value| PetId::parse(value).ok_or(PersistenceError::InvalidSelectedPet))
            .transpose()?;
        let window_placement = row
            .window_placement_json
            .map(|value| serde_json::from_str(value.as_str()))
            .transpose()?;
        let state = PersistentApplicationState::new(selected_pet_id, window_placement, reducer);
        state.validate()?;
        Ok(Some(state))
    }

    pub fn save(&self, state: &PersistentApplicationState) -> Result<(), PersistenceError> {
        state.validate()?;
        let reducer_json = json_document(&state.reducer)?;
        let reducer_value: serde_json::Value = serde_json::from_str(reducer_json.as_str())?;
        let reducer_revision = reducer_value
            .get("revision")
            .and_then(serde_json::Value::as_i64)
            .ok_or(PersistenceError::InvalidReducerSnapshot)?;
        let presentation_state = reducer_value
            .get("presentation")
            .and_then(serde_json::Value::as_str)
            .ok_or(PersistenceError::InvalidReducerSnapshot)?
            .to_owned();
        let presentation_since_ms = reducer_value
            .get("presentationSinceMs")
            .and_then(serde_json::Value::as_i64)
            .ok_or(PersistenceError::InvalidReducerSnapshot)?;
        let minimum_dwell_ms = reducer_value
            .get("minimumDwellMs")
            .and_then(serde_json::Value::as_i64)
            .ok_or(PersistenceError::InvalidReducerSnapshot)?;
        let window_placement_json = state
            .window_placement
            .as_ref()
            .map(json_document)
            .transpose()?;
        let selected_pet_id = state
            .selected_pet_id
            .as_ref()
            .map(|value| value.as_str().to_owned());
        let mut database = open(&self.paths)?;
        update_app_state(
            database.connection(),
            &lili_storage::models::AppStateRow {
                id: 1,
                schema_version: STORAGE_SCHEMA_VERSION,
                selected_pet_id,
                window_placement_json,
                reducer_json: Some(reducer_json),
                reducer_revision,
                presentation_state,
                presentation_since_ms,
                minimum_dwell_ms,
            },
        )?;
        Ok(())
    }
}

fn json_document<T: Serialize>(value: &T) -> Result<JsonDocument, PersistenceError> {
    JsonDocument::parse(serde_json::to_string(value)?)
        .map_err(PersistenceError::InvalidJsonDocument)
}

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("application state version {0} is unsupported")]
    UnsupportedVersion(u16),
    #[error("application state contains an invalid selected pet")]
    InvalidSelectedPet,
    #[error("application state contains an invalid window placement")]
    InvalidWindowPlacement,
    #[error("display work area is invalid")]
    InvalidDisplayWorkArea,
    #[error("application state contains invalid reducer metadata: {0}")]
    Reducer(#[from] ReducerRestoreError),
    #[error("application state contains an invalid reducer snapshot")]
    InvalidReducerSnapshot,
    #[error("application state contains invalid JSON: {0}")]
    InvalidJsonDocument(lili_storage::models::JsonDocumentError),
    #[error("application database operation failed: {0}")]
    Database(#[from] DatabaseError),
    #[error("application database query failed: {0}")]
    DatabaseQuery(#[from] diesel::result::Error),
    #[error("application state I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("application state is malformed: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("lili-app-state-{}-{sequence}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn state(pet_id: &str, logical_x: i32) -> PersistentApplicationState {
        PersistentApplicationState::new(
            PetId::parse(pet_id),
            Some(WindowPlacement::new("display-1", logical_x, 20, 2_000).unwrap()),
            SessionReducer::default().persistent_state(),
        )
    }

    #[test]
    fn state_round_trip_replaces_atomically() {
        let temp = TempDir::new();
        let paths = ApplicationPaths::from_root(temp.0.clone()).unwrap();
        let store = AppStateStore::for_application(paths.clone());
        store.save(&state("lili", 10)).unwrap();
        let replacement = state("custom-pet", 30);
        store.save(&replacement).unwrap();
        assert_eq!(store.load().unwrap(), Some(replacement));
        assert!(store.database_path().is_file());
        assert!(paths.root().is_dir());
    }

    #[test]
    fn logical_placement_converts_to_the_target_display_scale() {
        let saved = WindowPlacement::new("retina", 100, 50, 1_000).unwrap();
        let displays =
            [DisplayWorkArea::new("retina", 1_920, 40, 3_000, 2_000, 2_000, true).unwrap()];
        let resolved = resolve_window_placement(&saved, &displays, 640, 720, 48).unwrap();
        assert_eq!(resolved.physical_x(), 2_120);
        assert_eq!(resolved.physical_y(), 140);
        assert_eq!(resolved.placement().logical_x(), 100);
        assert_eq!(resolved.placement().scale_milli(), 2_000);
    }

    #[test]
    fn disconnected_display_falls_back_to_primary_and_remains_reachable() {
        let saved = WindowPlacement::new("removed", 10_000, -10_000, 1_000).unwrap();
        let displays = [
            DisplayWorkArea::new("secondary", -1_920, 0, 1_920, 1_080, 1_000, false).unwrap(),
            DisplayWorkArea::new("primary", 0, 24, 1_920, 1_056, 1_000, true).unwrap(),
        ];
        let resolved = resolve_window_placement(&saved, &displays, 320, 360, 48).unwrap();
        assert_eq!(resolved.placement().display_id(), "primary");
        assert_eq!(resolved.physical_x(), 1_872);
        assert_eq!(resolved.physical_y(), -288);
    }

    #[test]
    fn oversized_window_keeps_a_reachable_margin_on_small_work_area() {
        let saved = WindowPlacement::new("small", -5_000, 5_000, 1_000).unwrap();
        let displays = [DisplayWorkArea::new("small", 100, 200, 200, 100, 1_000, true).unwrap()];
        let resolved = resolve_window_placement(&saved, &displays, 600, 400, 48).unwrap();
        assert_eq!(resolved.physical_x(), -452);
        assert_eq!(resolved.physical_y(), 252);
    }
}
