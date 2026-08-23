mod database;
pub mod models;
pub mod repository;
pub mod schema;
pub mod transaction;

pub use database::{DatabaseError, EmbeddedDatabase, MIGRATIONS, connect, open};
pub use models::JsonDocument;

use std::{
    env, fmt, fs,
    path::{Path, PathBuf},
};

pub const APPLICATION_IDENTIFIER: &str = "dev.linw1995.lili";
const DATABASE_FILE_NAME: &str = "lili.sqlite3";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationPaths {
    root: PathBuf,
}

impl ApplicationPaths {
    pub fn resolve() -> Result<Self, PathError> {
        Self::from_root(platform_application_root()?)
    }

    pub fn from_root(root: impl Into<PathBuf>) -> Result<Self, PathError> {
        let root = root.into();
        if !root.is_absolute() {
            return Err(PathError::RelativeRoot(root));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn database_path(&self) -> PathBuf {
        self.root.join(DATABASE_FILE_NAME)
    }

    pub fn pets_root(&self) -> PathBuf {
        self.root.join("pets")
    }

    pub fn config_root(&self) -> PathBuf {
        self.root.join("config")
    }

    pub fn actions_path(&self) -> PathBuf {
        self.config_root().join("actions.toml")
    }

    pub fn runtime_root(&self) -> PathBuf {
        self.root.join("runtime")
    }

    pub fn credentials_path(&self) -> PathBuf {
        self.runtime_root().join("forwarding.json")
    }

    pub fn endpoint_path(&self) -> PathBuf {
        self.runtime_root().join("endpoint")
    }

    pub fn ensure_layout(&self) -> Result<(), StorageError> {
        for path in [
            self.root.clone(),
            self.config_root(),
            self.pets_root(),
            self.runtime_root(),
        ] {
            ensure_private_directory(&path)?;
        }
        for path in [
            self.database_path(),
            self.database_path().with_extension("sqlite3-wal"),
            self.database_path().with_extension("sqlite3-shm"),
            self.credentials_path(),
        ] {
            harden_existing_file(&path)?;
        }
        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum PathError {
    #[error("the user home directory is unavailable")]
    HomeDirectoryUnavailable,
    #[error("application root must be absolute: {0}")]
    RelativeRoot(PathBuf),
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("storage path is not a directory: {0}")]
    InvalidDirectory(PathBuf),
    #[error("storage path is not a regular file: {0}")]
    InvalidFile(PathBuf),
    #[error("storage I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

fn ensure_private_directory(path: &Path) -> Result<(), StorageError> {
    fs::create_dir_all(path).map_err(|source| StorageError::Io {
        path: path.to_owned(),
        source,
    })?;
    let metadata = fs::symlink_metadata(path).map_err(|source| StorageError::Io {
        path: path.to_owned(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StorageError::InvalidDirectory(path.to_owned()));
    }
    set_private_directory_mode(path).map_err(|source| StorageError::Io {
        path: path.to_owned(),
        source,
    })
}

fn harden_existing_file(path: &Path) -> Result<(), StorageError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(StorageError::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(StorageError::InvalidFile(path.to_owned()));
    }
    set_private_file_mode(path).map_err(|source| StorageError::Io {
        path: path.to_owned(),
        source,
    })
}

#[cfg(unix)]
fn set_private_directory_mode(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)
}

#[cfg(not(unix))]
fn set_private_directory_mode(_path: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_mode(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)
}

#[cfg(not(unix))]
fn set_private_file_mode(_path: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

fn platform_application_root() -> Result<PathBuf, PathError> {
    platform_application_root_from(|name| env::var_os(name))
}

fn platform_application_root_from(
    mut get_env: impl FnMut(&str) -> Option<std::ffi::OsString>,
) -> Result<PathBuf, PathError> {
    let base = platform_state_base(&mut get_env)?;
    Ok(base.join(APPLICATION_IDENTIFIER))
}

fn platform_state_base(
    get_env: &mut impl FnMut(&str) -> Option<std::ffi::OsString>,
) -> Result<PathBuf, PathError> {
    #[cfg(target_os = "linux")]
    {
        return Ok(non_empty_path(get_env, "XDG_STATE_HOME")
            .unwrap_or(home_directory(get_env)?.join(".local").join("state")));
    }

    #[cfg(target_os = "macos")]
    {
        return Ok(home_directory(get_env)?
            .join("Library")
            .join("Application Support"));
    }

    #[cfg(target_os = "windows")]
    {
        return Ok(non_empty_path(get_env, "LOCALAPPDATA")
            .unwrap_or(home_directory(get_env)?.join("AppData").join("Local")));
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Ok(home_directory(get_env)?.join(".local").join("state"))
    }
}

fn home_directory(
    get_env: &mut impl FnMut(&str) -> Option<std::ffi::OsString>,
) -> Result<PathBuf, PathError> {
    #[cfg(target_os = "windows")]
    let value = get_env("USERPROFILE").or_else(|| get_env("HOME"));
    #[cfg(not(target_os = "windows"))]
    let value = get_env("HOME");
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or(PathError::HomeDirectoryUnavailable)
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn non_empty_path(
    get_env: &mut impl FnMut(&str) -> Option<std::ffi::OsString>,
    name: &str,
) -> Option<PathBuf> {
    get_env(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

impl fmt::Display for ApplicationPaths {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.root.display().fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        ffi::OsString,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(0);

    fn temporary_root() -> PathBuf {
        let sequence = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("lili-storage-{}-{sequence}", std::process::id()))
    }

    #[test]
    fn application_layout_is_stable_from_an_explicit_root() {
        let paths = ApplicationPaths::from_root("/tmp/lili-storage").unwrap();

        assert_eq!(
            paths.database_path(),
            PathBuf::from("/tmp/lili-storage/lili.sqlite3")
        );
        assert_eq!(paths.pets_root(), PathBuf::from("/tmp/lili-storage/pets"));
        assert_eq!(
            paths.actions_path(),
            PathBuf::from("/tmp/lili-storage/config/actions.toml")
        );
        assert_eq!(
            paths.credentials_path(),
            PathBuf::from("/tmp/lili-storage/runtime/forwarding.json")
        );
        assert_eq!(
            paths.endpoint_path(),
            PathBuf::from("/tmp/lili-storage/runtime/endpoint")
        );
    }

    #[test]
    fn relative_roots_are_rejected() {
        assert!(matches!(
            ApplicationPaths::from_root("relative"),
            Err(PathError::RelativeRoot(_))
        ));
    }

    #[test]
    fn default_resolution_does_not_use_codex_home() {
        let paths = platform_application_root_from(|name| match name {
            "CODEX_HOME" => Some(OsString::from("/Users/example/Documents/codex")),
            "HOME" => Some(OsString::from("/Users/example")),
            _ => None,
        })
        .unwrap();

        assert!(!paths.starts_with("/Users/example/Documents/codex"));
        assert!(paths.is_absolute());
    }

    #[test]
    fn ensure_layout_creates_expected_directories() {
        let root = temporary_root();
        let paths = ApplicationPaths::from_root(&root).unwrap();

        paths.ensure_layout().unwrap();

        assert!(root.is_dir());
        assert!(paths.config_root().is_dir());
        assert!(paths.pets_root().is_dir());
        assert!(paths.runtime_root().is_dir());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn ensure_layout_rejects_a_symlinked_application_root() {
        use std::os::unix::fs::symlink;

        let root = temporary_root();
        let target = temporary_root();
        fs::create_dir_all(&target).unwrap();
        symlink(&target, &root).unwrap();
        let paths = ApplicationPaths::from_root(&root).unwrap();

        assert!(matches!(
            paths.ensure_layout(),
            Err(StorageError::InvalidDirectory(path)) if path == root
        ));
        fs::remove_file(root).unwrap();
        fs::remove_dir_all(target).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn ensure_layout_hardens_existing_credentials() {
        use std::os::unix::fs::PermissionsExt;

        let root = temporary_root();
        let paths = ApplicationPaths::from_root(&root).unwrap();
        paths.ensure_layout().unwrap();
        fs::write(paths.credentials_path(), b"secret").unwrap();
        fs::set_permissions(&paths.credentials_path(), fs::Permissions::from_mode(0o644)).unwrap();

        paths.ensure_layout().unwrap();

        assert_eq!(
            fs::metadata(paths.credentials_path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fs::remove_dir_all(root).unwrap();
    }
}
