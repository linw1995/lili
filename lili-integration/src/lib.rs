mod install;
mod plan;

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use lili_session::{CodexIntegrationSurface, TESTED_CODEX_VERSION};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use toml_edit::{DocumentMut, Item};

pub use install::{
    InstallError, InstallOutcome, InstalledFileProvenance, IntegrationProvenance, install,
    install_with_verifier, load_plan,
};
pub use plan::{
    InstallPlanStatus, IntegrationInstallMode, IntegrationInstallPlan, PlannedFileAction,
    PlannedFileChange, PlannedHookEntry, PlannedNotifyEntry, build_coexistence_install_plan,
    build_install_plan,
};

pub const INTEGRATION_SCHEMA_VERSION: u16 = 1;
pub const LILI_INTEGRATION_ID: &str = "lili-session-v1";
pub const CONFIG_FILE_NAME: &str = "config.toml";
pub const HOOKS_FILE_NAME: &str = "hooks.json";
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_VERSION_BYTES: usize = 128;
const HOOK_SURFACES: [(&str, CodexIntegrationSurface); 5] = [
    ("SessionStart", CodexIntegrationSurface::SessionStart),
    (
        "UserPromptSubmit",
        CodexIntegrationSurface::UserPromptSubmit,
    ),
    (
        "PermissionRequest",
        CodexIntegrationSurface::PermissionRequest,
    ),
    ("Stop", CodexIntegrationSurface::Stop),
    ("SessionEnd", CodexIntegrationSurface::SessionEnd),
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationFileStatus {
    Missing,
    Parsed,
    Malformed,
    TooLarge,
    Unreadable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationKind {
    Missing,
    Lili,
    Other,
    Invalid,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationFileInspection {
    pub path: PathBuf,
    pub status: IntegrationFileStatus,
    pub symbolic_link: bool,
    pub content_sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyInspection {
    pub kind: IntegrationKind,
    pub argument_count: usize,
    #[serde(skip)]
    pub(crate) argv: Option<Vec<String>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookSurfaceInspection {
    pub surface: CodexIntegrationSurface,
    pub lili_handlers: usize,
    pub other_handlers: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationInspection {
    pub schema_version: u16,
    pub codex_home: PathBuf,
    pub codex_version: Option<String>,
    pub tested_codex_version: String,
    pub config: IntegrationFileInspection,
    pub hooks: IntegrationFileInspection,
    pub notify: NotifyInspection,
    pub hook_surfaces: Vec<HookSurfaceInspection>,
    pub warnings: Vec<String>,
}

pub fn inspect(codex_home: &Path) -> IntegrationInspection {
    inspect_with_version(codex_home, detect_codex_version())
}

pub fn inspect_with_version(
    codex_home: &Path,
    codex_version: Option<String>,
) -> IntegrationInspection {
    let config_path = codex_home.join(CONFIG_FILE_NAME);
    let hooks_path = codex_home.join(HOOKS_FILE_NAME);
    let (config_file, config_document) = read_toml(&config_path);
    let (hooks_file, hooks_document) = read_json(&hooks_path);
    let notify = config_document
        .as_ref()
        .map_or_else(missing_notify, inspect_notify);
    let mut hook_surfaces = HOOK_SURFACES
        .into_iter()
        .map(|(name, surface)| {
            let (toml_lili, toml_other) = config_document
                .as_ref()
                .map_or((0, 0), |document| inspect_toml_hooks(document, name));
            let (json_lili, json_other) = hooks_document
                .as_ref()
                .map_or((0, 0), |document| inspect_json_hooks(document, name));
            HookSurfaceInspection {
                surface,
                lili_handlers: toml_lili.saturating_add(json_lili),
                other_handlers: toml_other.saturating_add(json_other),
            }
        })
        .collect::<Vec<_>>();
    hook_surfaces.sort_by_key(|surface| surface.surface);

    let mut warnings = Vec::new();
    if codex_version.is_none() {
        warnings.push("Codex version could not be detected.".to_owned());
    } else if codex_version.as_deref() != Some(TESTED_CODEX_VERSION) {
        warnings.push(
            "The installed Codex version differs from the tested adapter version.".to_owned(),
        );
    }
    if config_file.symbolic_link || hooks_file.symbolic_link {
        warnings
            .push("Symbolic-link configuration requires manual review before mutation.".to_owned());
    }
    if matches!(
        config_file.status,
        IntegrationFileStatus::Malformed
            | IntegrationFileStatus::TooLarge
            | IntegrationFileStatus::Unreadable
    ) || matches!(
        hooks_file.status,
        IntegrationFileStatus::Malformed
            | IntegrationFileStatus::TooLarge
            | IntegrationFileStatus::Unreadable
    ) {
        warnings.push("A configuration file could not be safely parsed.".to_owned());
    }

    IntegrationInspection {
        schema_version: INTEGRATION_SCHEMA_VERSION,
        codex_home: codex_home.to_path_buf(),
        codex_version,
        tested_codex_version: TESTED_CODEX_VERSION.to_owned(),
        config: config_file,
        hooks: hooks_file,
        notify,
        hook_surfaces,
        warnings,
    }
}

pub fn detect_codex_version() -> Option<String> {
    let output = Command::new("codex").arg("--version").output().ok()?;
    if !output.status.success() || output.stdout.len() > MAX_VERSION_BYTES {
        return None;
    }
    let output = std::str::from_utf8(&output.stdout).ok()?.trim();
    let version = output.split_whitespace().last()?;
    if version.is_empty()
        || version.len() > 64
        || version.chars().any(|character| character.is_control())
    {
        return None;
    }
    Some(version.to_owned())
}

fn read_toml(path: &Path) -> (IntegrationFileInspection, Option<DocumentMut>) {
    let (inspection, contents) = read_bounded(path);
    let Some(contents) = contents else {
        return (inspection, None);
    };
    match contents.parse::<DocumentMut>() {
        Ok(document) => (inspection, Some(document)),
        Err(_) => (
            IntegrationFileInspection {
                status: IntegrationFileStatus::Malformed,
                ..inspection
            },
            None,
        ),
    }
}

fn read_json(path: &Path) -> (IntegrationFileInspection, Option<serde_json::Value>) {
    let (inspection, contents) = read_bounded(path);
    let Some(contents) = contents else {
        return (inspection, None);
    };
    match serde_json::from_str::<serde_json::Value>(&contents) {
        Ok(document)
            if document
                .get("hooks")
                .is_none_or(serde_json::Value::is_object) =>
        {
            (inspection, Some(document))
        }
        Ok(_) | Err(_) => (
            IntegrationFileInspection {
                status: IntegrationFileStatus::Malformed,
                ..inspection
            },
            None,
        ),
    }
}

fn read_bounded(path: &Path) -> (IntegrationFileInspection, Option<String>) {
    let symbolic_link = fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false);
    let mut inspection = IntegrationFileInspection {
        path: path.to_path_buf(),
        status: IntegrationFileStatus::Missing,
        symbolic_link,
        content_sha256: None,
    };
    let metadata = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            inspection.status = IntegrationFileStatus::Unreadable;
            return (inspection, None);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (inspection, None),
        Err(_) => {
            inspection.status = IntegrationFileStatus::Unreadable;
            return (inspection, None);
        }
    };
    if metadata.len() > MAX_CONFIG_BYTES {
        inspection.status = IntegrationFileStatus::TooLarge;
        return (inspection, None);
    }
    match fs::read_to_string(path) {
        Ok(contents) if contents.len() as u64 <= MAX_CONFIG_BYTES => {
            inspection.status = IntegrationFileStatus::Parsed;
            inspection.content_sha256 = Some(sha256_hex(contents.as_bytes()));
            (inspection, Some(contents))
        }
        Ok(_) => {
            inspection.status = IntegrationFileStatus::TooLarge;
            (inspection, None)
        }
        Err(_) => {
            inspection.status = IntegrationFileStatus::Unreadable;
            (inspection, None)
        }
    }
}

pub(crate) fn sha256_hex(contents: &[u8]) -> String {
    let digest = Sha256::digest(contents);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn missing_notify() -> NotifyInspection {
    NotifyInspection {
        kind: IntegrationKind::Missing,
        argument_count: 0,
        argv: None,
    }
}

fn inspect_notify(document: &DocumentMut) -> NotifyInspection {
    let Some(item) = document.get("notify") else {
        return missing_notify();
    };
    let Some(array) = item.as_array() else {
        return NotifyInspection {
            kind: IntegrationKind::Invalid,
            argument_count: 0,
            argv: None,
        };
    };
    let values = array
        .iter()
        .map(|value| value.as_str())
        .collect::<Option<Vec<_>>>();
    let Some(values) = values else {
        return NotifyInspection {
            kind: IntegrationKind::Invalid,
            argument_count: array.len(),
            argv: None,
        };
    };
    let kind = if values
        .iter()
        .any(|value| value.contains(LILI_INTEGRATION_ID))
    {
        IntegrationKind::Lili
    } else {
        IntegrationKind::Other
    };
    NotifyInspection {
        kind,
        argument_count: values.len(),
        argv: Some(values.into_iter().map(str::to_owned).collect()),
    }
}

fn inspect_toml_hooks(document: &DocumentMut, surface: &str) -> (usize, usize) {
    let Some(groups) = document
        .get("hooks")
        .and_then(Item::as_table)
        .and_then(|hooks| hooks.get(surface))
        .and_then(Item::as_array_of_tables)
    else {
        return (0, 0);
    };
    classify_commands(groups.iter().flat_map(|group| {
        group
            .get("hooks")
            .and_then(Item::as_array_of_tables)
            .into_iter()
            .flatten()
            .filter_map(|handler| handler.get("command").and_then(Item::as_str))
    }))
}

fn inspect_json_hooks(document: &serde_json::Value, surface: &str) -> (usize, usize) {
    let commands = document
        .get("hooks")
        .and_then(|hooks| hooks.get(surface))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|group| {
            group
                .get("hooks")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|handler| handler.get("command").and_then(serde_json::Value::as_str));
    classify_commands(commands)
}

fn classify_commands<'a>(commands: impl IntoIterator<Item = &'a str>) -> (usize, usize) {
    commands
        .into_iter()
        .fold((0_usize, 0_usize), |(lili, other), command| {
            if command.contains(LILI_INTEGRATION_ID) {
                (lili.saturating_add(1), other)
            } else {
                (lili, other.saturating_add(1))
            }
        })
}

#[derive(Debug, Error)]
pub enum IntegrationError {
    #[error("the Codex home is not an absolute path")]
    RelativeCodexHome,
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
                "lili-integration-inspect-{}-{sequence}",
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
    fn inspection_classifies_integrations_without_exposing_values() {
        let temp = TempDir::new();
        fs::write(
            temp.0.join(CONFIG_FILE_NAME),
            r#"api_key = "do-not-report"
notify = ["other-notifier", "secret-argument"]
"#,
        )
        .unwrap();
        fs::write(
            temp.0.join(HOOKS_FILE_NAME),
            r#"{
  "hooks": {
    "SessionStart": [{
      "hooks": [
        {"type": "command", "command": "other-hook --token secret"},
        {"type": "command", "command": "lili-hook --integration-id lili-session-v1 --json-stdin"}
      ]
    }]
  }
}"#,
        )
        .unwrap();

        let inspection = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
        assert_eq!(inspection.notify.kind, IntegrationKind::Other);
        let session_start = inspection
            .hook_surfaces
            .iter()
            .find(|surface| surface.surface == CodexIntegrationSurface::SessionStart)
            .unwrap();
        assert_eq!(session_start.lili_handlers, 1);
        assert_eq!(session_start.other_handlers, 1);
        let serialized = serde_json::to_string(&inspection).unwrap();
        assert!(!serialized.contains("do-not-report"));
        assert!(!serialized.contains("secret-argument"));
        assert!(!serialized.contains("other-hook"));
    }

    #[test]
    fn malformed_and_missing_files_are_reported_without_aborting_inspection() {
        let temp = TempDir::new();
        fs::write(temp.0.join(CONFIG_FILE_NAME), "notify = [").unwrap();
        let inspection = inspect_with_version(&temp.0, None);
        assert_eq!(inspection.config.status, IntegrationFileStatus::Malformed);
        assert_eq!(inspection.hooks.status, IntegrationFileStatus::Missing);
        assert_eq!(inspection.notify.kind, IntegrationKind::Missing);
        assert!(!inspection.warnings.is_empty());
    }
}
