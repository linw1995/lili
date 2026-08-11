use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use lili_core::PetId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    AtlasMetadata, AtlasValidationError, DEFAULT_PET_ID, PetDefinition, SPRITE_VERSION_NUMBER,
    ValidatedPetPackage, discover_pet_packages, validate_discovered_package,
    validation::{read_validated_atlas, validate_atlas_bytes},
};

const SELECTION_VERSION: u8 = 1;
const MAX_SELECTION_BYTES: u64 = 4 * 1024;
const FALLBACK_MANIFEST: &str = include_str!("../assets/fallback/pet.json");
const FALLBACK_ATLAS: &[u8] = include_bytes!("../assets/fallback/spritesheet.webp");
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PetAssetSource {
    File(PathBuf),
    Embedded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AvailablePet {
    definition: PetDefinition,
    atlas: AtlasMetadata,
    source: PetAssetSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedPetAsset {
    bytes: Vec<u8>,
    atlas: AtlasMetadata,
}

impl LoadedPetAsset {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn atlas(&self) -> AtlasMetadata {
        self.atlas
    }
}

impl AvailablePet {
    pub const fn definition(&self) -> &PetDefinition {
        &self.definition
    }

    pub const fn atlas(&self) -> AtlasMetadata {
        self.atlas
    }

    pub const fn source(&self) -> &PetAssetSource {
        &self.source
    }

    pub fn embedded_atlas(&self) -> Option<&'static [u8]> {
        matches!(self.source, PetAssetSource::Embedded).then_some(FALLBACK_ATLAS)
    }

    pub fn load_asset(&self) -> Result<LoadedPetAsset, AtlasValidationError> {
        let (bytes, atlas) = match &self.source {
            PetAssetSource::File(path) => read_validated_atlas(path)?,
            PetAssetSource::Embedded => {
                let bytes = FALLBACK_ATLAS.to_vec();
                let atlas = validate_atlas_bytes(&bytes)?;
                (bytes, atlas)
            }
        };
        Ok(LoadedPetAsset { bytes, atlas })
    }
}

impl From<ValidatedPetPackage> for AvailablePet {
    fn from(package: ValidatedPetPackage) -> Self {
        Self {
            definition: package.definition().clone(),
            atlas: package.atlas(),
            source: PetAssetSource::File(package.atlas_path().to_owned()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogDiagnostic {
    package_dir: Option<PathBuf>,
    message: String,
}

impl CatalogDiagnostic {
    pub fn package_dir(&self) -> Option<&Path> {
        self.package_dir.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PetCatalog {
    active: AvailablePet,
    packages: Vec<AvailablePet>,
    requested_identifier: String,
    diagnostics: Vec<CatalogDiagnostic>,
}

impl Default for PetCatalog {
    fn default() -> Self {
        Self {
            active: fallback_pet(),
            packages: Vec::new(),
            requested_identifier: DEFAULT_PET_ID.to_owned(),
            diagnostics: Vec::new(),
        }
    }
}

impl PetCatalog {
    pub fn load(codex_home: &Path) -> Self {
        let mut diagnostics = Vec::new();
        let requested_identifier = match load_selected_pet(codex_home) {
            Ok(Some(identifier)) => identifier,
            Ok(None) => DEFAULT_PET_ID.to_owned(),
            Err(error) => {
                diagnostics.push(CatalogDiagnostic {
                    package_dir: None,
                    message: format!("selection state was ignored: {error}"),
                });
                DEFAULT_PET_ID.to_owned()
            }
        };
        Self::load_requested(codex_home, requested_identifier, diagnostics)
    }

    pub fn load_with_selection(codex_home: &Path, selected: Option<&PetId>) -> Self {
        let requested_identifier = selected.map_or(DEFAULT_PET_ID, PetId::as_str).to_owned();
        Self::load_requested(codex_home, requested_identifier, Vec::new())
    }

    fn load_requested(
        codex_home: &Path,
        requested_identifier: String,
        mut diagnostics: Vec<CatalogDiagnostic>,
    ) -> Self {
        let discovered = discover_pet_packages(codex_home);
        diagnostics.extend(discovered.issues().iter().map(|issue| CatalogDiagnostic {
            package_dir: Some(issue.package_dir().to_owned()),
            message: issue.message().to_owned(),
        }));

        let mut packages = Vec::new();
        for package in discovered.packages() {
            match validate_discovered_package(package) {
                Ok(package) => packages.push(AvailablePet::from(package)),
                Err(error) => diagnostics.push(CatalogDiagnostic {
                    package_dir: Some(package.package_dir().to_owned()),
                    message: error.to_string(),
                }),
            }
        }

        let fallback = fallback_pet();
        let active = packages
            .iter()
            .find(|pet| pet.definition().id().as_str() == requested_identifier)
            .cloned()
            .unwrap_or_else(|| {
                if requested_identifier != DEFAULT_PET_ID {
                    diagnostics.push(CatalogDiagnostic {
                        package_dir: None,
                        message: format!(
                            "selected pet `{requested_identifier}` is unavailable; using embedded fallback"
                        ),
                    });
                }
                fallback
            });

        Self {
            active,
            packages,
            requested_identifier,
            diagnostics,
        }
    }

    pub const fn active(&self) -> &AvailablePet {
        &self.active
    }

    pub fn packages(&self) -> &[AvailablePet] {
        &self.packages
    }

    pub fn requested_identifier(&self) -> &str {
        &self.requested_identifier
    }

    pub fn diagnostics(&self) -> &[CatalogDiagnostic] {
        &self.diagnostics
    }
}

fn fallback_pet() -> AvailablePet {
    let manifest: crate::PetManifest =
        serde_json::from_str(FALLBACK_MANIFEST).expect("embedded fallback manifest must be valid");
    assert_eq!(
        manifest.sprite_version_number(),
        SPRITE_VERSION_NUMBER,
        "embedded fallback must use the v2 manifest"
    );
    AvailablePet {
        definition: PetDefinition {
            id: PetId::parse(manifest.id()).expect("embedded fallback id must be valid"),
            display_name: manifest.display_name().to_owned(),
            description: manifest.description().to_owned(),
        },
        atlas: validate_atlas_bytes(FALLBACK_ATLAS)
            .expect("embedded fallback atlas must pass v2 validation"),
        source: PetAssetSource::Embedded,
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredSelection {
    version: u8,
    pet_id: String,
}

pub fn selection_path(codex_home: &Path) -> PathBuf {
    codex_home.join("lili").join("selected-pet.json")
}

fn load_selected_pet(codex_home: &Path) -> Result<Option<String>, SelectionError> {
    let path = selection_path(codex_home);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SelectionError::InvalidFile);
    }
    if metadata.len() > MAX_SELECTION_BYTES {
        return Err(SelectionError::TooLarge);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)?
        .take(MAX_SELECTION_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SELECTION_BYTES {
        return Err(SelectionError::TooLarge);
    }
    let selection: StoredSelection = serde_json::from_slice(&bytes)?;
    if selection.version != SELECTION_VERSION {
        return Err(SelectionError::UnsupportedVersion(selection.version));
    }
    PetId::parse(&selection.pet_id).ok_or(SelectionError::InvalidIdentifier)?;
    Ok(Some(selection.pet_id))
}

pub fn persist_selected_pet(codex_home: &Path, pet_id: &PetId) -> Result<(), SelectionError> {
    let path = selection_path(codex_home);
    let directory = path.parent().expect("selection path has a parent");
    fs::create_dir_all(directory)?;
    let metadata = fs::symlink_metadata(directory)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SelectionError::InvalidDirectory);
    }

    let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(
        ".selected-pet-{}-{sequence}.tmp",
        std::process::id()
    ));
    let selection = StoredSelection {
        version: SELECTION_VERSION,
        pet_id: pet_id.as_str().to_owned(),
    };
    let mut payload = serde_json::to_vec(&selection)?;
    payload.push(b'\n');

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
    fs::rename(&temporary, path)?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum SelectionError {
    #[error("selection state must be a regular file")]
    InvalidFile,
    #[error("selection state directory must not be a symlink")]
    InvalidDirectory,
    #[error("selection state exceeds 4 KiB")]
    TooLarge,
    #[error("selection state version {0} is unsupported")]
    UnsupportedVersion(u8),
    #[error("selection state contains an invalid pet identifier")]
    InvalidIdentifier,
    #[error("selection state I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("selection state is malformed: {0}")]
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
            let path = std::env::temp_dir().join(format!(
                "lili-pet-selection-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_user_package_uses_embedded_fallback() {
        let temp = TempDir::new();
        let catalog = PetCatalog::load(&temp.0);
        assert_eq!(catalog.active().definition().id().as_str(), DEFAULT_PET_ID);
        assert_eq!(catalog.active().source(), &PetAssetSource::Embedded);
        assert!(catalog.active().embedded_atlas().is_some());
    }

    #[test]
    fn fixed_default_package_takes_precedence_over_embedded_fallback() {
        let temp = TempDir::new();
        let package_dir = temp.0.join("pet").join(DEFAULT_PET_ID);
        fs::create_dir_all(&package_dir).unwrap();
        fs::write(package_dir.join("pet.json"), FALLBACK_MANIFEST).unwrap();
        fs::write(package_dir.join("spritesheet.webp"), FALLBACK_ATLAS).unwrap();

        let catalog = PetCatalog::load(&temp.0);

        assert_eq!(catalog.active().definition().id().as_str(), DEFAULT_PET_ID);
        assert!(matches!(catalog.active().source(), PetAssetSource::File(_)));
        assert!(catalog.active().embedded_atlas().is_none());
    }

    #[test]
    fn missing_selected_identifier_is_preserved_for_diagnostics() {
        let temp = TempDir::new();
        let missing = PetId::parse("missing").unwrap();
        persist_selected_pet(&temp.0, &missing).unwrap();
        let catalog = PetCatalog::load(&temp.0);
        assert_eq!(catalog.requested_identifier(), "missing");
        assert_eq!(catalog.active().source(), &PetAssetSource::Embedded);
        assert!(
            catalog
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.message().contains("unavailable"))
        );
    }

    #[test]
    fn selection_round_trip_is_atomic_and_identifier_only() {
        let temp = TempDir::new();
        let selected = PetId::parse("cat-2").unwrap();
        persist_selected_pet(&temp.0, &selected).unwrap();
        assert_eq!(
            load_selected_pet(&temp.0).unwrap(),
            Some("cat-2".to_owned())
        );
        let payload = fs::read_to_string(selection_path(&temp.0)).unwrap();
        assert!(!payload.contains('/'));
    }

    #[test]
    fn explicit_application_state_selection_overrides_legacy_selection_file() {
        let temp = TempDir::new();
        let legacy = PetId::parse("missing").unwrap();
        persist_selected_pet(&temp.0, &legacy).unwrap();
        let selected = PetId::parse(DEFAULT_PET_ID).unwrap();
        let catalog = PetCatalog::load_with_selection(&temp.0, Some(&selected));
        assert_eq!(catalog.requested_identifier(), DEFAULT_PET_ID);
        assert!(catalog.diagnostics().is_empty());
    }
}
