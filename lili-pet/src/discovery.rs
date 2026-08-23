use std::{
    collections::HashSet,
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};

use lili_core::PetId;
use thiserror::Error;

use crate::{PetManifest, SPRITE_VERSION_NUMBER};

pub const DEFAULT_PET_ID: &str = "lili";
pub const MAX_PET_MANIFEST_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageOrigin {
    Default,
    Installed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredPackage {
    origin: PackageOrigin,
    package_dir: PathBuf,
    atlas_path: PathBuf,
    manifest: PetManifest,
}

impl DiscoveredPackage {
    pub const fn origin(&self) -> PackageOrigin {
        self.origin
    }

    pub fn package_dir(&self) -> &Path {
        &self.package_dir
    }

    pub fn atlas_path(&self) -> &Path {
        &self.atlas_path
    }

    pub const fn manifest(&self) -> &PetManifest {
        &self.manifest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryIssue {
    package_dir: PathBuf,
    message: String,
}

impl DiscoveryIssue {
    pub fn package_dir(&self) -> &Path {
        &self.package_dir
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DiscoveryReport {
    packages: Vec<DiscoveredPackage>,
    issues: Vec<DiscoveryIssue>,
}

impl DiscoveryReport {
    pub fn packages(&self) -> &[DiscoveredPackage] {
        &self.packages
    }

    pub fn issues(&self) -> &[DiscoveryIssue] {
        &self.issues
    }
}

pub fn default_pet_path(pets_root: &Path) -> PathBuf {
    pets_root.join(DEFAULT_PET_ID)
}

pub fn discover_pet_packages(pets_root: &Path) -> DiscoveryReport {
    let mut candidates = Vec::new();
    let default = default_pet_path(pets_root);
    if default.exists() {
        candidates.push((PackageOrigin::Default, default.clone()));
    }

    if let Ok(entries) = fs::read_dir(pets_root) {
        let mut installed = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path() != default)
            .map(|entry| (PackageOrigin::Installed, entry.path()))
            .collect::<Vec<_>>();
        installed.sort_by(|left, right| left.1.cmp(&right.1));
        candidates.extend(installed);
    }

    let mut report = DiscoveryReport::default();
    let mut identifiers = HashSet::new();
    for (origin, package_dir) in candidates {
        match load_package(origin, &package_dir) {
            Ok(package) => {
                if identifiers.insert(package.manifest.id().to_owned()) {
                    report.packages.push(package);
                } else {
                    report
                        .issues
                        .push(issue(package_dir, "duplicate pet identifier"));
                }
            }
            Err(error) => report.issues.push(issue(package_dir, error.to_string())),
        }
    }
    report
}

fn load_package(
    origin: PackageOrigin,
    package_dir: &Path,
) -> Result<DiscoveredPackage, PackageLoadError> {
    let package_metadata = fs::symlink_metadata(package_dir)?;
    if package_metadata.file_type().is_symlink() || !package_metadata.is_dir() {
        return Err(PackageLoadError::InvalidPackageDirectory);
    }
    let package_dir = package_dir.canonicalize()?;
    let manifest_path = package_dir.join("pet.json");
    let manifest_metadata = fs::symlink_metadata(&manifest_path)?;
    if manifest_metadata.file_type().is_symlink() || !manifest_metadata.is_file() {
        return Err(PackageLoadError::InvalidManifestFile);
    }
    if manifest_metadata.len() > MAX_PET_MANIFEST_BYTES as u64 {
        return Err(PackageLoadError::ManifestTooLarge);
    }

    let mut bytes = Vec::with_capacity(manifest_metadata.len() as usize);
    File::open(&manifest_path)?
        .take(MAX_PET_MANIFEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let manifest = parse_pet_manifest(&bytes).map_err(PackageLoadError::Manifest)?;

    let relative_asset = Path::new(manifest.spritesheet_path());
    if relative_asset.as_os_str().is_empty()
        || relative_asset.is_absolute()
        || relative_asset
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(PackageLoadError::InvalidAssetPath);
    }

    let unresolved_asset = package_dir.join(relative_asset);
    let asset_metadata = fs::symlink_metadata(&unresolved_asset)?;
    if !asset_metadata.file_type().is_file() && !asset_metadata.file_type().is_symlink() {
        return Err(PackageLoadError::InvalidAssetFile);
    }
    let atlas_path = unresolved_asset.canonicalize()?;
    if !atlas_path.starts_with(&package_dir) || !atlas_path.is_file() {
        return Err(PackageLoadError::EscapingAssetPath);
    }

    Ok(DiscoveredPackage {
        origin,
        package_dir,
        atlas_path,
        manifest,
    })
}

pub fn parse_pet_manifest(payload: &[u8]) -> Result<PetManifest, PetManifestError> {
    if payload.len() > MAX_PET_MANIFEST_BYTES {
        return Err(PetManifestError::TooLarge);
    }
    let manifest = serde_json::from_slice(payload).map_err(|_| PetManifestError::Malformed)?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &PetManifest) -> Result<(), PetManifestError> {
    PetId::parse(manifest.id()).ok_or(PetManifestError::InvalidIdentifier)?;
    if manifest.display_name().trim().is_empty() || manifest.display_name().len() > 128 {
        return Err(PetManifestError::InvalidDisplayName);
    }
    if manifest.description().len() > 512 {
        return Err(PetManifestError::InvalidDescription);
    }
    if manifest.sprite_version_number() != SPRITE_VERSION_NUMBER {
        return Err(PetManifestError::UnsupportedVersion);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PetManifestError {
    #[error("pet.json exceeds 64 KiB")]
    TooLarge,
    #[error("pet.json is malformed")]
    Malformed,
    #[error("pet identifier is invalid")]
    InvalidIdentifier,
    #[error("display name is empty or exceeds 128 bytes")]
    InvalidDisplayName,
    #[error("description exceeds 512 bytes")]
    InvalidDescription,
    #[error("spriteVersionNumber must be 2")]
    UnsupportedVersion,
}

fn issue(package_dir: PathBuf, message: impl Into<String>) -> DiscoveryIssue {
    DiscoveryIssue {
        package_dir,
        message: message.into(),
    }
}

#[derive(Debug, Error)]
enum PackageLoadError {
    #[error("package directory must be a real directory")]
    InvalidPackageDirectory,
    #[error("pet.json must be a real file")]
    InvalidManifestFile,
    #[error("pet.json exceeds 64 KiB")]
    ManifestTooLarge,
    #[error("{0}")]
    Manifest(PetManifestError),
    #[error("spritesheetPath must be a confined relative path")]
    InvalidAssetPath,
    #[error("spritesheetPath must resolve to a file")]
    InvalidAssetFile,
    #[error("spritesheetPath resolves outside its package")]
    EscapingAssetPath,
    #[error("package I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "lili-pet-discovery-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_package(root: &Path, directory: &str, id: &str, asset_path: &str) -> PathBuf {
        let package = root.join("pets").join(directory);
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("spritesheet.webp"), b"fixture").unwrap();
        fs::write(
            package.join("pet.json"),
            format!(
                r#"{{"id":"{id}","displayName":"Pet","description":"Fixture","spriteVersionNumber":2,"spritesheetPath":"{asset_path}"}}"#
            ),
        )
        .unwrap();
        package
    }

    #[test]
    fn default_pet_uses_shared_pets_directory() {
        assert_eq!(
            default_pet_path(Path::new("/tmp/lili/pets")),
            PathBuf::from("/tmp/lili/pets/lili")
        );
    }

    #[test]
    fn legacy_singular_pet_directory_is_not_discovered() {
        let temp = TempDir::new();
        let package = temp.path().join("pet").join(DEFAULT_PET_ID);
        fs::create_dir_all(&package).unwrap();
        fs::write(package.join("spritesheet.webp"), b"fixture").unwrap();
        fs::write(
            package.join("pet.json"),
            r#"{"id":"lili","displayName":"Lili","description":"Legacy","spriteVersionNumber":2,"spritesheetPath":"spritesheet.webp"}"#,
        )
        .unwrap();

        let report = discover_pet_packages(&temp.path().join("pets"));

        assert!(report.packages().is_empty());
        assert!(report.issues().is_empty());
    }

    #[test]
    fn discovers_valid_package() {
        let temp = TempDir::new();
        write_package(temp.path(), "valid", "valid", "spritesheet.webp");
        let report = discover_pet_packages(&temp.path().join("pets"));
        assert_eq!(report.packages().len(), 1);
        assert!(report.issues().is_empty());
        assert_eq!(report.packages()[0].manifest().id(), "valid");
    }

    #[test]
    fn rejects_absolute_and_traversing_assets() {
        let temp = TempDir::new();
        write_package(temp.path(), "absolute", "absolute", "/tmp/atlas.webp");
        write_package(temp.path(), "traversal", "traversal", "../atlas.webp");
        let report = discover_pet_packages(&temp.path().join("pets"));
        assert!(report.packages().is_empty());
        assert_eq!(report.issues().len(), 2);
        assert!(
            report
                .issues()
                .iter()
                .all(|issue| issue.message().contains("confined relative path"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escaping_package() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new();
        let outside = temp.path().join("outside.webp");
        fs::write(&outside, b"fixture").unwrap();
        let package = write_package(temp.path(), "linked", "linked", "linked.webp");
        symlink(outside, package.join("linked.webp")).unwrap();
        let report = discover_pet_packages(&temp.path().join("pets"));
        assert!(report.packages().is_empty());
        assert_eq!(report.issues().len(), 1);
        assert!(report.issues()[0].message().contains("outside"));
    }

    #[test]
    fn rejects_duplicate_identifiers() {
        let temp = TempDir::new();
        write_package(temp.path(), "first", "duplicate", "spritesheet.webp");
        write_package(temp.path(), "second", "duplicate", "spritesheet.webp");
        let report = discover_pet_packages(&temp.path().join("pets"));
        assert_eq!(report.packages().len(), 1);
        assert_eq!(report.issues().len(), 1);
        assert!(report.issues()[0].message().contains("duplicate"));
    }

    #[test]
    fn rejects_oversized_manifest() {
        let temp = TempDir::new();
        let package = write_package(temp.path(), "large", "large", "spritesheet.webp");
        fs::write(package.join("pet.json"), vec![b' '; 64 * 1024 + 1]).unwrap();
        let report = discover_pet_packages(&temp.path().join("pets"));
        assert!(report.packages().is_empty());
        assert_eq!(report.issues().len(), 1);
        assert!(report.issues()[0].message().contains("64 KiB"));
    }
}
