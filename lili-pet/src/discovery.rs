use std::{
    collections::HashSet,
    env,
    ffi::OsString,
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};

use lili_core::PetId;
use thiserror::Error;

use crate::{PetManifest, SPRITE_VERSION_NUMBER};

pub const DEFAULT_PET_ID: &str = "lili";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

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

#[derive(Debug, Error)]
pub enum CodexHomeError {
    #[error("CODEX_HOME must be an absolute path")]
    RelativeOverride,
    #[error("unable to resolve the current user's home directory")]
    MissingHome,
}

pub fn resolve_codex_home() -> Result<PathBuf, CodexHomeError> {
    resolve_codex_home_from(|name| env::var_os(name))
}

fn resolve_codex_home_from(
    mut get_env: impl FnMut(&str) -> Option<OsString>,
) -> Result<PathBuf, CodexHomeError> {
    if let Some(value) = get_env("CODEX_HOME").filter(|value| !value.is_empty()) {
        let path = PathBuf::from(value);
        return path
            .is_absolute()
            .then_some(path)
            .ok_or(CodexHomeError::RelativeOverride);
    }

    let home = get_env("HOME")
        .or_else(|| get_env("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or(CodexHomeError::MissingHome)?;
    Ok(home.join(".codex"))
}

pub fn default_pet_path(codex_home: &Path) -> PathBuf {
    codex_home.join("pet").join(DEFAULT_PET_ID)
}

pub fn discover_pet_packages(codex_home: &Path) -> DiscoveryReport {
    let mut candidates = Vec::new();
    let default = default_pet_path(codex_home);
    if default.exists() {
        candidates.push((PackageOrigin::Default, default));
    }

    let installed_root = codex_home.join("pets");
    if let Ok(entries) = fs::read_dir(installed_root) {
        let mut installed = entries
            .filter_map(Result::ok)
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
    if manifest_metadata.len() > MAX_MANIFEST_BYTES {
        return Err(PackageLoadError::ManifestTooLarge);
    }

    let mut bytes = Vec::with_capacity(manifest_metadata.len() as usize);
    File::open(&manifest_path)?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(PackageLoadError::ManifestTooLarge);
    }
    let manifest: PetManifest = serde_json::from_slice(&bytes)?;
    validate_manifest(&manifest)?;

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

fn validate_manifest(manifest: &PetManifest) -> Result<(), PackageLoadError> {
    PetId::parse(manifest.id()).ok_or(PackageLoadError::InvalidIdentifier)?;
    if manifest.display_name().trim().is_empty() || manifest.display_name().len() > 128 {
        return Err(PackageLoadError::InvalidDisplayName);
    }
    if manifest.description().len() > 512 {
        return Err(PackageLoadError::InvalidDescription);
    }
    if manifest.sprite_version_number() != SPRITE_VERSION_NUMBER {
        return Err(PackageLoadError::UnsupportedVersion);
    }
    Ok(())
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
    #[error("pet identifier is invalid")]
    InvalidIdentifier,
    #[error("display name is empty or exceeds 128 bytes")]
    InvalidDisplayName,
    #[error("description exceeds 512 bytes")]
    InvalidDescription,
    #[error("spriteVersionNumber must be 2")]
    UnsupportedVersion,
    #[error("spritesheetPath must be a confined relative path")]
    InvalidAssetPath,
    #[error("spritesheetPath must resolve to a file")]
    InvalidAssetFile,
    #[error("spritesheetPath resolves outside its package")]
    EscapingAssetPath,
    #[error("package I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("pet.json is malformed: {0}")]
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
    fn codex_home_prefers_absolute_override() {
        let result = resolve_codex_home_from(|name| match name {
            "CODEX_HOME" => Some(OsString::from("/tmp/custom-codex")),
            "HOME" => Some(OsString::from("/tmp/home")),
            _ => None,
        });
        assert_eq!(result.unwrap(), PathBuf::from("/tmp/custom-codex"));
    }

    #[test]
    fn default_pet_uses_singular_pet_directory() {
        assert_eq!(
            default_pet_path(Path::new("/tmp/codex")),
            PathBuf::from("/tmp/codex/pet/lili")
        );
    }

    #[test]
    fn discovers_valid_package() {
        let temp = TempDir::new();
        write_package(temp.path(), "valid", "valid", "spritesheet.webp");
        let report = discover_pet_packages(temp.path());
        assert_eq!(report.packages().len(), 1);
        assert!(report.issues().is_empty());
        assert_eq!(report.packages()[0].manifest().id(), "valid");
    }

    #[test]
    fn rejects_absolute_and_traversing_assets() {
        let temp = TempDir::new();
        write_package(temp.path(), "absolute", "absolute", "/tmp/atlas.webp");
        write_package(temp.path(), "traversal", "traversal", "../atlas.webp");
        let report = discover_pet_packages(temp.path());
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
        let report = discover_pet_packages(temp.path());
        assert!(report.packages().is_empty());
        assert_eq!(report.issues().len(), 1);
        assert!(report.issues()[0].message().contains("outside"));
    }

    #[test]
    fn rejects_duplicate_identifiers() {
        let temp = TempDir::new();
        write_package(temp.path(), "first", "duplicate", "spritesheet.webp");
        write_package(temp.path(), "second", "duplicate", "spritesheet.webp");
        let report = discover_pet_packages(temp.path());
        assert_eq!(report.packages().len(), 1);
        assert_eq!(report.issues().len(), 1);
        assert!(report.issues()[0].message().contains("duplicate"));
    }

    #[test]
    fn rejects_oversized_manifest() {
        let temp = TempDir::new();
        let package = write_package(temp.path(), "large", "large", "spritesheet.webp");
        fs::write(package.join("pet.json"), vec![b' '; 64 * 1024 + 1]).unwrap();
        let report = discover_pet_packages(temp.path());
        assert!(report.packages().is_empty());
        assert_eq!(report.issues().len(), 1);
        assert!(report.issues()[0].message().contains("64 KiB"));
    }
}
