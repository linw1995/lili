use std::{
    env, fmt,
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
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum PathError {
    #[error("the user home directory is unavailable")]
    HomeDirectoryUnavailable,
    #[error("application root must be absolute: {0}")]
    RelativeRoot(PathBuf),
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
    use std::ffi::OsString;

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
}
