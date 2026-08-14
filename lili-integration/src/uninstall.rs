use std::{fs, path::Path, path::PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use toml_edit::{Array, DocumentMut, value};

use crate::{
    CONFIG_FILE_NAME, HOOKS_FILE_NAME, IntegrationProvenance, LILI_INTEGRATION_ID,
    install::{InstallError, atomic_write, provenance_path, remove_file, rollback_file},
    sha256_hex,
};

const MAX_PROVENANCE_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallOutcome {
    pub changed: bool,
    pub complete: bool,
    pub restored_notify: bool,
    pub removed_hook_handlers: usize,
    pub conflicts: Vec<String>,
    pub provenance: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallPreview {
    pub complete: bool,
    pub restored_notify: bool,
    pub removed_hook_handlers: usize,
    pub conflicts: Vec<String>,
}

pub fn preview_uninstall(codex_home: &Path) -> Result<UninstallPreview, UninstallError> {
    let provenance_path = provenance_path(codex_home);
    let provenance: IntegrationProvenance =
        serde_json::from_slice(&read_required_bounded(&provenance_path)?)
            .map_err(|_| UninstallError::InvalidProvenance)?;
    validate_provenance(codex_home, &provenance)?;
    let config_path = codex_home.join(CONFIG_FILE_NAME);
    let hooks_path = codex_home.join(HOOKS_FILE_NAME);
    reject_symbolic_link(&config_path)?;
    reject_symbolic_link(&hooks_path)?;
    let mut conflicts = Vec::new();
    let (_, restored_notify, config_complete) = plan_config_uninstall(
        &provenance,
        read_optional(&config_path)?.as_deref(),
        &mut conflicts,
    )?;
    let (_, removed_hook_handlers, hooks_complete) = plan_hooks_uninstall(
        &provenance,
        read_optional(&hooks_path)?.as_deref(),
        &mut conflicts,
    )?;
    Ok(UninstallPreview {
        complete: config_complete && hooks_complete,
        restored_notify,
        removed_hook_handlers,
        conflicts,
    })
}

pub fn uninstall(codex_home: &Path) -> Result<UninstallOutcome, UninstallError> {
    let provenance_path = provenance_path(codex_home);
    let original_provenance = read_required_bounded(&provenance_path)?;
    let provenance: IntegrationProvenance = serde_json::from_slice(&original_provenance)
        .map_err(|_| UninstallError::InvalidProvenance)?;
    validate_provenance(codex_home, &provenance)?;

    let config_path = codex_home.join(CONFIG_FILE_NAME);
    let hooks_path = codex_home.join(HOOKS_FILE_NAME);
    reject_symbolic_link(&config_path)?;
    reject_symbolic_link(&hooks_path)?;
    let original_config = read_optional(&config_path)?;
    let original_hooks = read_optional(&hooks_path)?;
    let mut conflicts = Vec::new();
    let (config_mutation, restored_notify, config_complete) =
        plan_config_uninstall(&provenance, original_config.as_deref(), &mut conflicts)?;
    let (hooks_mutation, removed_hook_handlers, hooks_complete) =
        plan_hooks_uninstall(&provenance, original_hooks.as_deref(), &mut conflicts)?;
    let complete = config_complete && hooks_complete;
    let changed = !matches!(config_mutation, FileMutation::Unchanged)
        || !matches!(hooks_mutation, FileMutation::Unchanged)
        || complete;

    let apply_result: Result<(), InstallError> = (|| {
        apply_mutation(&config_path, &config_mutation)?;
        apply_mutation(&hooks_path, &hooks_mutation)?;
        if complete {
            remove_file(&provenance_path)?;
        }
        Ok(())
    })();
    if let Err(error) = apply_result {
        rollback_file(&config_path, original_config.as_deref())?;
        rollback_file(&hooks_path, original_hooks.as_deref())?;
        rollback_file(&provenance_path, Some(&original_provenance))?;
        return Err(error.into());
    }

    Ok(UninstallOutcome {
        changed,
        complete,
        restored_notify,
        removed_hook_handlers,
        conflicts,
        provenance: (!complete).then_some(provenance_path),
    })
}

#[derive(Debug)]
enum FileMutation {
    Unchanged,
    Write(Vec<u8>),
    Remove,
}

fn validate_provenance(
    codex_home: &Path,
    provenance: &IntegrationProvenance,
) -> Result<(), UninstallError> {
    if provenance.schema_version != 1
        || provenance.integration_id != LILI_INTEGRATION_ID
        || !codex_home.is_absolute()
        || provenance.config.path != codex_home.join(CONFIG_FILE_NAME)
        || provenance.hooks.path != codex_home.join(HOOKS_FILE_NAME)
        || provenance.notify_argv.is_empty()
        || provenance.hook_commands.is_empty()
        || !provenance
            .notify_argv
            .iter()
            .any(|argument| argument.contains(LILI_INTEGRATION_ID))
        || provenance
            .hook_commands
            .iter()
            .any(|command| !command.contains(LILI_INTEGRATION_ID))
    {
        return Err(UninstallError::InvalidProvenance);
    }
    Ok(())
}

fn plan_config_uninstall(
    provenance: &IntegrationProvenance,
    contents: Option<&[u8]>,
    conflicts: &mut Vec<String>,
) -> Result<(FileMutation, bool, bool), UninstallError> {
    let Some(contents) = contents else {
        return Ok((FileMutation::Unchanged, false, true));
    };
    let text = std::str::from_utf8(contents).map_err(|_| UninstallError::InvalidConfiguration)?;
    let mut document = text
        .parse::<DocumentMut>()
        .map_err(|_| UninstallError::InvalidConfiguration)?;
    let current_notify = document
        .get("notify")
        .and_then(|item| item.as_array())
        .and_then(|array| {
            array
                .iter()
                .map(|value| value.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        });
    if current_notify.as_deref() != Some(provenance.notify_argv.as_slice()) {
        let contains_marker = current_notify.as_ref().is_some_and(|argv| {
            argv.iter()
                .any(|argument| argument.contains(LILI_INTEGRATION_ID))
        });
        conflicts.push("notify changed after Lili installation and was left unchanged".to_owned());
        return Ok((FileMutation::Unchanged, false, !contains_marker));
    }

    if provenance.config.created
        && provenance.previous_notify_argv.is_none()
        && sha256_hex(contents) == provenance.config.after_sha256
    {
        return Ok((FileMutation::Remove, false, true));
    }
    let restored_notify = if let Some(previous) = &provenance.previous_notify_argv {
        let mut notify = Array::new();
        for argument in previous {
            notify.push(argument.as_str());
        }
        document["notify"] = value(notify);
        true
    } else {
        document.remove("notify");
        false
    };
    Ok((
        FileMutation::Write(document.to_string().into_bytes()),
        restored_notify,
        true,
    ))
}

fn plan_hooks_uninstall(
    provenance: &IntegrationProvenance,
    contents: Option<&[u8]>,
    conflicts: &mut Vec<String>,
) -> Result<(FileMutation, usize, bool), UninstallError> {
    let Some(contents) = contents else {
        return Ok((FileMutation::Unchanged, 0, true));
    };
    let mut document = serde_json::from_slice::<serde_json::Value>(contents)
        .map_err(|_| UninstallError::InvalidConfiguration)?;
    let Some(hooks) = document.get("hooks").and_then(serde_json::Value::as_object) else {
        let complete = !String::from_utf8_lossy(contents).contains(LILI_INTEGRATION_ID);
        if !complete {
            conflicts.push("Lili hook markers could not be structurally removed".to_owned());
        }
        return Ok((FileMutation::Unchanged, 0, complete));
    };
    let managed_hook_changed = hooks
        .values()
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(serde_json::Value::as_array))
        .flatten()
        .filter(|handler| is_lili_handler(handler))
        .any(|handler| !matches_hook_provenance(handler, provenance));
    if managed_hook_changed {
        conflicts.push("Lili hooks changed after installation and were left unchanged".to_owned());
        return Ok((FileMutation::Unchanged, 0, false));
    }

    let hooks = document
        .get_mut("hooks")
        .and_then(serde_json::Value::as_object_mut)
        .expect("hooks object was validated above");
    let mut removed = 0_usize;
    for groups in hooks
        .values_mut()
        .filter_map(serde_json::Value::as_array_mut)
    {
        for group in groups.iter_mut() {
            let Some(handlers) = group
                .get_mut("hooks")
                .and_then(serde_json::Value::as_array_mut)
            else {
                continue;
            };
            let before = handlers.len();
            handlers.retain(|handler| !is_lili_handler(handler));
            removed = removed.saturating_add(before.saturating_sub(handlers.len()));
        }
        groups.retain(|group| {
            group
                .get("hooks")
                .and_then(serde_json::Value::as_array)
                .is_none_or(|handlers| !handlers.is_empty())
        });
    }
    hooks.retain(|_, groups| groups.as_array().is_none_or(|groups| !groups.is_empty()));
    if removed == 0 {
        return Ok((FileMutation::Unchanged, 0, true));
    }
    if provenance.hooks.created && sha256_hex(contents) == provenance.hooks.after_sha256 {
        return Ok((FileMutation::Remove, removed, true));
    }
    let mut rendered = serde_json::to_vec_pretty(&document)?;
    rendered.push(b'\n');
    Ok((FileMutation::Write(rendered), removed, true))
}

fn is_lili_handler(handler: &serde_json::Value) -> bool {
    ["command", "commandWindows"].iter().any(|key| {
        handler
            .get(key)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|command| command.contains(LILI_INTEGRATION_ID))
    })
}

fn matches_hook_provenance(
    handler: &serde_json::Value,
    provenance: &IntegrationProvenance,
) -> bool {
    let Some(handler) = handler.as_object() else {
        return false;
    };
    let expected_fields = [
        "type",
        "command",
        "commandWindows",
        "timeout",
        "statusMessage",
    ];
    if handler.len() != expected_fields.len()
        || expected_fields
            .iter()
            .any(|field| !handler.contains_key(*field))
    {
        return false;
    }
    let command_matches = handler
        .get("command")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|command| {
            provenance
                .hook_commands
                .iter()
                .any(|recorded| recorded == command)
        });
    let quoted_windows_binary = provenance
        .hook_binary
        .display()
        .to_string()
        .replace('"', "\\\"");
    let expected_windows_command =
        format!("\"{quoted_windows_binary}\" --integration-id {LILI_INTEGRATION_ID} --json-stdin");
    command_matches
        && handler.get("type").and_then(serde_json::Value::as_str) == Some("command")
        && handler
            .get("commandWindows")
            .and_then(serde_json::Value::as_str)
            == Some(expected_windows_command.as_str())
        // Provenance predates timeout recording, so accept only the two Lili-generated shapes.
        && matches!(
            handler.get("timeout").and_then(serde_json::Value::as_u64),
            Some(1 | 2)
        )
        && handler
            .get("statusMessage")
            .and_then(serde_json::Value::as_str)
            == Some("Notify Lili")
}

fn apply_mutation(path: &Path, mutation: &FileMutation) -> Result<(), InstallError> {
    match mutation {
        FileMutation::Unchanged => Ok(()),
        FileMutation::Write(contents) => atomic_write(path, contents),
        FileMutation::Remove => remove_file(path),
    }
}

fn read_required_bounded(path: &Path) -> Result<Vec<u8>, UninstallError> {
    let metadata = fs::metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            UninstallError::NotInstalled
        } else {
            UninstallError::Io(error)
        }
    })?;
    if !metadata.is_file() || metadata.len() > MAX_PROVENANCE_BYTES {
        return Err(UninstallError::InvalidProvenance);
    }
    fs::read(path).map_err(UninstallError::Io)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, UninstallError> {
    match fs::read(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn reject_symbolic_link(path: &Path) -> Result<(), UninstallError> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(UninstallError::InvalidConfiguration);
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum UninstallError {
    #[error("Lili integration is not installed")]
    NotInstalled,
    #[error("integration provenance is invalid")]
    InvalidProvenance,
    #[error("configuration could not be safely edited")]
    InvalidConfiguration,
    #[error("integration I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("integration JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("integration mutation failed: {0}")]
    Install(#[from] InstallError),
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::{
        IntegrationInstallMode, build_coexistence_install_plan, build_install_plan,
        inspect_with_version, install_with_verifier,
    };

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "lili-integration-uninstall-{}-{sequence}",
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

    fn install_exclusive(temp: &TempDir, timestamp: u64) {
        let inspection = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
        let plan = build_install_plan(&inspection, &temp.0.join("bin/lili-hook"), timestamp);
        install_with_verifier(&plan, |_| Ok(())).unwrap();
    }

    #[test]
    fn exclusive_uninstall_removes_created_files_but_keeps_pet_packages() {
        let temp = TempDir::new();
        let pet = temp.0.join("pets/lili/pet.json");
        fs::create_dir_all(pet.parent().unwrap()).unwrap();
        fs::write(&pet, "{}\n").unwrap();
        install_exclusive(&temp, 42);

        let outcome = uninstall(&temp.0).unwrap();
        assert!(outcome.changed);
        assert!(outcome.complete);
        assert_eq!(outcome.removed_hook_handlers, 5);
        assert!(!temp.0.join(CONFIG_FILE_NAME).exists());
        assert!(!temp.0.join(HOOKS_FILE_NAME).exists());
        assert!(!provenance_path(&temp.0).exists());
        assert_eq!(fs::read_to_string(pet).unwrap(), "{}\n");
    }

    #[test]
    fn coexistence_uninstall_restores_notify_and_preserves_other_hooks() {
        let temp = TempDir::new();
        fs::write(
            temp.0.join(CONFIG_FILE_NAME),
            "model = \"gpt-5\"\nnotify = [\"existing\", \"--channel\", \"pet\"]\n",
        )
        .unwrap();
        fs::write(
            temp.0.join(HOOKS_FILE_NAME),
            r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"existing-hook"}]}]}}"#,
        )
        .unwrap();
        let inspection = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
        let plan = build_coexistence_install_plan(&inspection, &temp.0.join("bin/lili-hook"), 42);
        install_with_verifier(&plan, |_| Ok(())).unwrap();

        let outcome = uninstall(&temp.0).unwrap();
        assert!(outcome.complete);
        assert!(outcome.restored_notify);
        assert!(outcome.conflicts.is_empty());
        let config = fs::read_to_string(temp.0.join(CONFIG_FILE_NAME)).unwrap();
        assert!(config.contains("model = \"gpt-5\""));
        assert!(config.contains("notify = [\"existing\", \"--channel\", \"pet\"]"));
        let hooks = fs::read_to_string(temp.0.join(HOOKS_FILE_NAME)).unwrap();
        assert!(hooks.contains("existing-hook"));
        assert!(!hooks.contains(LILI_INTEGRATION_ID));
    }

    #[test]
    fn uninstall_preserves_configuration_modified_after_install() {
        let temp = TempDir::new();
        fs::write(temp.0.join(CONFIG_FILE_NAME), "model = \"gpt-5\"\n").unwrap();
        install_exclusive(&temp, 42);
        let config = "model = \"gpt-5.1\"\nnotify = [\"replacement\", \"--new\"]\n";
        fs::write(temp.0.join(CONFIG_FILE_NAME), config).unwrap();
        let mut hooks: serde_json::Value =
            serde_json::from_slice(&fs::read(temp.0.join(HOOKS_FILE_NAME)).unwrap()).unwrap();
        hooks["hooks"]["SessionStart"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "hooks": [{"type": "command", "command": "new-hook"}]
            }));
        fs::write(
            temp.0.join(HOOKS_FILE_NAME),
            serde_json::to_vec_pretty(&hooks).unwrap(),
        )
        .unwrap();

        let outcome = uninstall(&temp.0).unwrap();
        assert!(outcome.complete);
        assert_eq!(outcome.conflicts.len(), 1);
        assert_eq!(
            fs::read_to_string(temp.0.join(CONFIG_FILE_NAME)).unwrap(),
            config
        );
        let hooks = fs::read_to_string(temp.0.join(HOOKS_FILE_NAME)).unwrap();
        assert!(hooks.contains("new-hook"));
        assert!(!hooks.contains(LILI_INTEGRATION_ID));
        assert!(!provenance_path(&temp.0).exists());
    }

    #[test]
    fn changed_managed_notify_keeps_provenance_for_manual_resolution() {
        let temp = TempDir::new();
        install_exclusive(&temp, 42);
        fs::write(
            temp.0.join(CONFIG_FILE_NAME),
            format!("notify = [\"custom\", \"--integration-id\", \"{LILI_INTEGRATION_ID}\"]\n"),
        )
        .unwrap();
        let outcome = uninstall(&temp.0).unwrap();
        assert!(!outcome.complete);
        assert_eq!(outcome.provenance, Some(provenance_path(&temp.0)));
        assert!(provenance_path(&temp.0).exists());
        assert!(
            fs::read_to_string(temp.0.join(CONFIG_FILE_NAME))
                .unwrap()
                .contains("custom")
        );
    }

    #[test]
    fn changed_managed_hook_blocks_uninstall_preview() {
        let temp = TempDir::new();
        install_exclusive(&temp, 42);
        let hooks_path = temp.0.join(HOOKS_FILE_NAME);
        let mut hooks: serde_json::Value =
            serde_json::from_slice(&fs::read(&hooks_path).unwrap()).unwrap();
        let command = hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .to_owned();
        hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"] =
            serde_json::Value::String(format!("{command} --custom"));
        fs::write(&hooks_path, serde_json::to_vec_pretty(&hooks).unwrap()).unwrap();

        let preview = preview_uninstall(&temp.0).unwrap();
        assert!(!preview.complete);
        assert_eq!(preview.removed_hook_handlers, 0);
        assert_eq!(
            preview.conflicts,
            vec!["Lili hooks changed after installation and were left unchanged"]
        );
    }

    #[test]
    fn legacy_one_second_hooks_remain_cleanup_compatible() {
        let temp = TempDir::new();
        install_exclusive(&temp, 42);
        let hooks_path = temp.0.join(HOOKS_FILE_NAME);
        let mut hooks: serde_json::Value =
            serde_json::from_slice(&fs::read(&hooks_path).unwrap()).unwrap();
        for groups in hooks["hooks"].as_object_mut().unwrap().values_mut() {
            for group in groups.as_array_mut().unwrap() {
                for handler in group["hooks"].as_array_mut().unwrap() {
                    handler["timeout"] = serde_json::json!(1);
                }
            }
        }
        fs::write(&hooks_path, serde_json::to_vec_pretty(&hooks).unwrap()).unwrap();

        let preview = preview_uninstall(&temp.0).unwrap();
        assert!(preview.complete);
        assert_eq!(preview.removed_hook_handlers, 5);
    }

    #[test]
    fn repeated_install_retains_original_uninstall_provenance() {
        let temp = TempDir::new();
        install_exclusive(&temp, 42);
        install_exclusive(&temp, 43);
        let provenance: IntegrationProvenance =
            serde_json::from_slice(&fs::read(provenance_path(&temp.0)).unwrap()).unwrap();
        assert_eq!(provenance.mode, IntegrationInstallMode::Exclusive);
        assert!(provenance.config.created);
        assert!(provenance.hooks.created);
        let outcome = uninstall(&temp.0).unwrap();
        assert!(outcome.complete);
        assert!(!temp.0.join(CONFIG_FILE_NAME).exists());
        assert!(!temp.0.join(HOOKS_FILE_NAME).exists());
    }
}
