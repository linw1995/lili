use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use lili_core::PetId;
use lili_session::{ReducerRestoreError, SessionReducer, SessionReducerState};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const PERSISTENCE_VERSION: u16 = 1;
const MAX_PERSISTED_STATE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_DISPLAY_ID_CHARS: usize = 256;
const MAX_LOGICAL_COORDINATE: i32 = 1_000_000;
const MIN_SCALE_MILLI: u32 = 250;
const MAX_SCALE_MILLI: u32 = 8_000;
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

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
    path: PathBuf,
}

impl AppStateStore {
    pub fn for_codex_home(codex_home: &Path) -> Self {
        Self {
            path: codex_home.join("lili").join("state.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Option<PersistentApplicationState>, PersistenceError> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(PersistenceError::InvalidFile);
        }
        if metadata.len() > MAX_PERSISTED_STATE_BYTES {
            return Err(PersistenceError::TooLarge);
        }
        let mut payload = Vec::with_capacity(metadata.len() as usize);
        File::open(&self.path)?
            .take(MAX_PERSISTED_STATE_BYTES + 1)
            .read_to_end(&mut payload)?;
        if payload.len() as u64 > MAX_PERSISTED_STATE_BYTES {
            return Err(PersistenceError::TooLarge);
        }
        let header: VersionHeader = serde_json::from_slice(&payload)?;
        if header.version != PERSISTENCE_VERSION {
            return Err(PersistenceError::UnsupportedVersion(header.version));
        }
        let state: PersistentApplicationState = serde_json::from_slice(&payload)?;
        state.validate()?;
        Ok(Some(state))
    }

    pub fn save(&self, state: &PersistentApplicationState) -> Result<(), PersistenceError> {
        state.validate()?;
        let mut payload = serde_json::to_vec(state)?;
        payload.push(b'\n');
        if payload.len() as u64 > MAX_PERSISTED_STATE_BYTES {
            return Err(PersistenceError::TooLarge);
        }

        let directory = self
            .path
            .parent()
            .expect("application state path must have a parent");
        fs::create_dir_all(directory)?;
        let metadata = fs::symlink_metadata(directory)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(PersistenceError::InvalidDirectory);
        }

        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(".state-{}-{sequence}.tmp", std::process::id()));
        let mut guard = TemporaryFileGuard::new(temporary.clone());
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&payload)?;
        file.sync_all()?;
        fs::rename(&temporary, &self.path)?;
        guard.commit();
        sync_directory(directory)?;
        Ok(())
    }
}

#[derive(Deserialize)]
struct VersionHeader {
    version: u16,
}

struct TemporaryFileGuard {
    path: PathBuf,
    committed: bool,
}

impl TemporaryFileGuard {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TemporaryFileGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(unix)]
fn sync_directory(directory: &Path) -> Result<(), std::io::Error> {
    File::open(directory)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("application state must be a regular file")]
    InvalidFile,
    #[error("application state directory must not be a symlink")]
    InvalidDirectory,
    #[error("application state exceeds 2 MiB")]
    TooLarge,
    #[error("application state version {0} is unsupported")]
    UnsupportedVersion(u16),
    #[error("application state contains an invalid selected pet")]
    InvalidSelectedPet,
    #[error("application state contains an invalid window placement")]
    InvalidWindowPlacement,
    #[error("application state contains invalid reducer metadata: {0}")]
    Reducer(#[from] ReducerRestoreError),
    #[error("application state I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("application state is malformed: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
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
        let store = AppStateStore::for_codex_home(&temp.0);
        store.save(&state("lili", 10)).unwrap();
        let replacement = state("custom-pet", 30);
        store.save(&replacement).unwrap();
        assert_eq!(store.load().unwrap(), Some(replacement));
        let directory = store.path().parent().unwrap();
        assert!(fs::read_dir(directory).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }

    #[test]
    fn unsupported_version_is_rejected_before_state_restore() {
        let temp = TempDir::new();
        let store = AppStateStore::for_codex_home(&temp.0);
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(store.path(), br#"{"version":2}"#).unwrap();
        assert!(matches!(
            store.load(),
            Err(PersistenceError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn oversized_state_is_rejected_before_json_parsing() {
        let temp = TempDir::new();
        let store = AppStateStore::for_codex_home(&temp.0);
        fs::create_dir_all(store.path().parent().unwrap()).unwrap();
        fs::write(
            store.path(),
            vec![b'x'; MAX_PERSISTED_STATE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(matches!(store.load(), Err(PersistenceError::TooLarge)));
    }
}
