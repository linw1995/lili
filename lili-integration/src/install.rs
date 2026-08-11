use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use toml_edit::{Array, DocumentMut, value};

use crate::{
    HOOKS_FILE_NAME, InstallPlanStatus, IntegrationInstallMode, IntegrationInstallPlan,
    IntegrationKind, LILI_INTEGRATION_ID, PlannedFileAction, build_coexistence_install_plan,
    build_install_plan, inspect_with_version, sha256_hex,
};

const PROVENANCE_FILE_NAME: &str = "integration.json";
const MAX_PLAN_BYTES: u64 = 256 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledFileProvenance {
    pub path: PathBuf,
    pub created: bool,
    pub before_sha256: Option<String>,
    pub after_sha256: String,
    pub backup: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationProvenance {
    pub schema_version: u16,
    pub integration_id: String,
    pub installed_at_ms: u64,
    pub hook_binary: PathBuf,
    pub mode: IntegrationInstallMode,
    pub notify_argv: Vec<String>,
    pub previous_notify_argv: Option<Vec<String>>,
    pub hook_commands: Vec<String>,
    pub config: InstalledFileProvenance,
    pub hooks: InstalledFileProvenance,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOutcome {
    pub changed: bool,
    pub backups: Vec<PathBuf>,
    pub provenance: PathBuf,
    pub verification: &'static str,
}

pub fn load_plan(path: &Path) -> Result<IntegrationInstallPlan, InstallError> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() > MAX_PLAN_BYTES {
        return Err(InstallError::InvalidPlan);
    }
    let mut contents = Vec::with_capacity(metadata.len() as usize);
    File::open(path)?
        .take(MAX_PLAN_BYTES + 1)
        .read_to_end(&mut contents)?;
    if contents.len() as u64 > MAX_PLAN_BYTES {
        return Err(InstallError::InvalidPlan);
    }
    serde_json::from_slice(&contents).map_err(|_| InstallError::InvalidPlan)
}

pub fn install(plan: &IntegrationInstallPlan) -> Result<InstallOutcome, InstallError> {
    install_with_verifier(plan, run_verification)
}

pub fn install_with_verifier<F>(
    plan: &IntegrationInstallPlan,
    verifier: F,
) -> Result<InstallOutcome, InstallError>
where
    F: FnOnce(&[String]) -> Result<(), InstallError>,
{
    validate_plan(plan)?;
    let original_config = read_expected(
        &plan.config_change.target,
        plan.config_change.expected_content_sha256.as_deref(),
    )?;
    let original_hooks = read_expected(
        &plan.hooks_change.target,
        plan.hooks_change.expected_content_sha256.as_deref(),
    )?;
    validate_current_plan(plan)?;
    let rendered_config = render_config(plan, original_config.as_deref())?;
    let rendered_hooks = render_hooks(plan, original_hooks.as_deref())?;
    let changed = rendered_config.is_some() || rendered_hooks.is_some();
    let provenance_path = provenance_path(&plan.codex_home);
    let original_provenance = match fs::read(&provenance_path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let previous_provenance = original_provenance
        .as_deref()
        .and_then(|contents| serde_json::from_slice::<IntegrationProvenance>(contents).ok())
        .filter(|provenance| provenance.integration_id == LILI_INTEGRATION_ID);

    let mut backups = Vec::new();
    create_backup_if_needed(
        &plan.config_change,
        original_config.as_deref(),
        &mut backups,
    )?;
    create_backup_if_needed(&plan.hooks_change, original_hooks.as_deref(), &mut backups)?;

    let apply_result = (|| {
        if let Some(contents) = &rendered_config {
            atomic_write(&plan.config_change.target, contents)?;
        }
        if let Some(contents) = &rendered_hooks {
            atomic_write(&plan.hooks_change.target, contents)?;
        }
        verify_installed_markers(plan)?;
        verifier(&plan.verification_command)?;
        let provenance = build_provenance(
            plan,
            original_config.as_deref(),
            original_hooks.as_deref(),
            rendered_config.as_deref(),
            rendered_hooks.as_deref(),
            previous_provenance.as_ref(),
        )?;
        atomic_write_json(&provenance_path, &provenance)?;
        Ok(provenance_path.clone())
    })();

    let provenance = match apply_result {
        Ok(provenance) => provenance,
        Err(error) => {
            rollback_file(&plan.config_change.target, original_config.as_deref())?;
            rollback_file(&plan.hooks_change.target, original_hooks.as_deref())?;
            rollback_file(&provenance_path, original_provenance.as_deref())?;
            return Err(error);
        }
    };

    Ok(InstallOutcome {
        changed,
        backups,
        provenance,
        verification: "accepted",
    })
}

fn validate_plan(plan: &IntegrationInstallPlan) -> Result<(), InstallError> {
    if plan.status != InstallPlanStatus::Ready
        || !plan.codex_home.is_absolute()
        || !plan.hook_binary.is_absolute()
        || plan.config_change.target != plan.codex_home.join(crate::CONFIG_FILE_NAME)
        || plan.hooks_change.target != plan.codex_home.join(HOOKS_FILE_NAME)
        || plan.notify.argv.first().map(Path::new) != Some(plan.hook_binary.as_path())
        || plan.notify.argv.get(1).map(String::as_str) != Some("--integration-id")
        || plan.notify.argv.get(2).map(String::as_str) != Some(LILI_INTEGRATION_ID)
        || plan.verification_command.first() != plan.notify.argv.first()
    {
        return Err(InstallError::InvalidPlan);
    }
    match (plan.mode, plan.previous_notify_argv.as_deref()) {
        (IntegrationInstallMode::Exclusive, None)
            if plan.notify.argv.len() == 4
                && plan.notify.argv.get(3).map(String::as_str) == Some("--json-argv") => {}
        (IntegrationInstallMode::Coexist, Some(previous))
            if !previous.is_empty()
                && plan.notify.argv.len() == 5
                && plan.notify.argv.get(3).map(String::as_str) == Some("--coexist-notify-json")
                && plan.notify.argv.get(4) == serde_json::to_string(previous).ok().as_ref() => {}
        (IntegrationInstallMode::Coexist, None)
            if plan.notify.argv.len() == 4
                && plan.notify.argv.get(3).map(String::as_str) == Some("--json-argv") => {}
        _ => return Err(InstallError::InvalidPlan),
    }
    Ok(())
}

fn validate_current_plan(plan: &IntegrationInstallPlan) -> Result<(), InstallError> {
    let inspection = inspect_with_version(&plan.codex_home, plan.codex_version.clone());
    let expected = match plan.mode {
        IntegrationInstallMode::Exclusive => {
            build_install_plan(&inspection, &plan.hook_binary, plan_timestamp(plan)?)
        }
        IntegrationInstallMode::Coexist => {
            build_coexistence_install_plan(&inspection, &plan.hook_binary, plan_timestamp(plan)?)
        }
    };
    if expected != *plan {
        return Err(InstallError::InvalidPlan);
    }
    Ok(())
}

fn read_expected(
    path: &Path,
    expected_hash: Option<&str>,
) -> Result<Option<Vec<u8>>, InstallError> {
    match fs::read(path) {
        Ok(contents) => {
            if expected_hash != Some(sha256_hex(&contents).as_str()) {
                return Err(InstallError::ConfigurationChanged);
            }
            Ok(Some(contents))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if expected_hash.is_some() {
                Err(InstallError::ConfigurationChanged)
            } else {
                Ok(None)
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn render_config(
    plan: &IntegrationInstallPlan,
    original: Option<&[u8]>,
) -> Result<Option<Vec<u8>>, InstallError> {
    if !matches!(
        plan.config_change.action,
        PlannedFileAction::Create | PlannedFileAction::Update
    ) {
        return Ok(None);
    }
    let mut document = match original {
        Some(contents) => std::str::from_utf8(contents)
            .map_err(|_| InstallError::InvalidConfiguration)?
            .parse::<DocumentMut>()
            .map_err(|_| InstallError::InvalidConfiguration)?,
        None => DocumentMut::new(),
    };
    let mut notify = Array::new();
    for argument in &plan.notify.argv {
        notify.push(argument.as_str());
    }
    document["notify"] = value(notify);
    Ok(Some(document.to_string().into_bytes()))
}

fn render_hooks(
    plan: &IntegrationInstallPlan,
    original: Option<&[u8]>,
) -> Result<Option<Vec<u8>>, InstallError> {
    if !matches!(
        plan.hooks_change.action,
        PlannedFileAction::Create | PlannedFileAction::Update
    ) {
        return Ok(None);
    }
    let mut document = match original {
        Some(contents) => serde_json::from_slice::<serde_json::Value>(contents)
            .map_err(|_| InstallError::InvalidConfiguration)?,
        None => serde_json::json!({"hooks": {}}),
    };
    let hooks = document
        .as_object_mut()
        .ok_or(InstallError::InvalidConfiguration)?
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or(InstallError::InvalidConfiguration)?;
    for addition in &plan.hook_additions {
        let groups = hooks
            .entry(addition.event_name.clone())
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .ok_or(InstallError::InvalidConfiguration)?;
        groups.push(serde_json::json!({
            "hooks": [{
                "type": addition.handler_type,
                "command": addition.command,
                "commandWindows": addition.command_windows,
                "timeout": addition.timeout_seconds,
                "statusMessage": "Notify Lili"
            }]
        }));
    }
    let mut rendered = serde_json::to_vec_pretty(&document)?;
    rendered.push(b'\n');
    Ok(Some(rendered))
}

fn create_backup_if_needed(
    change: &crate::PlannedFileChange,
    original: Option<&[u8]>,
    backups: &mut Vec<PathBuf>,
) -> Result<(), InstallError> {
    let Some(backup) = &change.backup else {
        return Ok(());
    };
    let contents = original.ok_or(InstallError::InvalidPlan)?;
    write_new_private_file(backup, contents)?;
    if let Some(parent) = backup.parent() {
        sync_directory(parent)?;
    }
    backups.push(backup.clone());
    Ok(())
}

fn verify_installed_markers(plan: &IntegrationInstallPlan) -> Result<(), InstallError> {
    let inspection = inspect_with_version(&plan.codex_home, plan.codex_version.clone());
    if inspection.notify.kind != IntegrationKind::Lili
        || inspection
            .hook_surfaces
            .iter()
            .any(|surface| surface.lili_handlers == 0)
    {
        return Err(InstallError::VerificationFailed);
    }
    Ok(())
}

fn run_verification(command: &[String]) -> Result<(), InstallError> {
    let Some(program) = command.first() else {
        return Err(InstallError::InvalidPlan);
    };
    let output = Command::new(program).args(&command[1..]).output()?;
    if !output.status.success() || !output.stdout.is_empty() {
        return Err(InstallError::VerificationFailed);
    }
    Ok(())
}

fn build_provenance(
    plan: &IntegrationInstallPlan,
    original_config: Option<&[u8]>,
    original_hooks: Option<&[u8]>,
    rendered_config: Option<&[u8]>,
    rendered_hooks: Option<&[u8]>,
    previous: Option<&IntegrationProvenance>,
) -> Result<IntegrationProvenance, InstallError> {
    let current_config = rendered_config
        .map(Vec::from)
        .or_else(|| original_config.map(Vec::from))
        .ok_or(InstallError::InvalidPlan)?;
    let current_hooks = rendered_hooks
        .map(Vec::from)
        .or_else(|| original_hooks.map(Vec::from))
        .ok_or(InstallError::InvalidPlan)?;
    let installed_at_ms = plan_timestamp(plan)?;
    let hook_commands = managed_hook_commands(&current_hooks)?;
    let config = if rendered_config.is_none() {
        previous.map(|provenance| provenance.config.clone())
    } else {
        None
    }
    .unwrap_or_else(|| InstalledFileProvenance {
        path: plan.config_change.target.clone(),
        created: original_config.is_none(),
        before_sha256: original_config.map(sha256_hex),
        after_sha256: sha256_hex(&current_config),
        backup: plan.config_change.backup.clone(),
    });
    let hooks = if rendered_hooks.is_none() {
        previous.map(|provenance| provenance.hooks.clone())
    } else {
        None
    }
    .unwrap_or_else(|| InstalledFileProvenance {
        path: plan.hooks_change.target.clone(),
        created: original_hooks.is_none(),
        before_sha256: original_hooks.map(sha256_hex),
        after_sha256: sha256_hex(&current_hooks),
        backup: plan.hooks_change.backup.clone(),
    });
    Ok(IntegrationProvenance {
        schema_version: 1,
        integration_id: LILI_INTEGRATION_ID.to_owned(),
        installed_at_ms,
        hook_binary: plan.hook_binary.clone(),
        mode: plan.mode,
        notify_argv: plan.notify.argv.clone(),
        previous_notify_argv: plan.previous_notify_argv.clone(),
        hook_commands,
        config,
        hooks,
    })
}

fn plan_timestamp(plan: &IntegrationInstallPlan) -> Result<u64, InstallError> {
    plan.verification_command
        .last()
        .and_then(|payload| serde_json::from_str::<serde_json::Value>(payload).ok())
        .and_then(|payload| payload.get("turn-id")?.as_str()?.parse().ok())
        .ok_or(InstallError::InvalidPlan)
}

fn managed_hook_commands(contents: &[u8]) -> Result<Vec<String>, InstallError> {
    let document: serde_json::Value = serde_json::from_slice(contents)?;
    let mut commands = document
        .get("hooks")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flat_map(|hooks| hooks.values())
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .filter_map(|group| group.get("hooks").and_then(serde_json::Value::as_array))
        .flatten()
        .filter_map(|handler| handler.get("command").and_then(serde_json::Value::as_str))
        .filter(|command| command.contains(LILI_INTEGRATION_ID))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    commands.sort();
    commands.dedup();
    Ok(commands)
}

pub(crate) fn provenance_path(codex_home: &Path) -> PathBuf {
    codex_home.join("lili").join(PROVENANCE_FILE_NAME)
}

fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), InstallError> {
    let mut contents = serde_json::to_vec_pretty(value)?;
    contents.push(b'\n');
    atomic_write(path, &contents)
}

pub(crate) fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), InstallError> {
    let parent = path.parent().ok_or(InstallError::InvalidPlan)?;
    ensure_private_directory(parent)?;
    let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    let temporary = parent.join(format!(
        ".{file_name}.lili-{}-{sequence}.tmp",
        std::process::id()
    ));
    let result = (|| {
        write_new_private_file(&temporary, contents)?;
        fs::rename(&temporary, path)?;
        sync_directory(parent)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn write_new_private_file(path: &Path, contents: &[u8]) -> Result<(), InstallError> {
    let parent = path.parent().ok_or(InstallError::InvalidPlan)?;
    fs::create_dir_all(parent)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

pub(crate) fn rollback_file(path: &Path, original: Option<&[u8]>) -> Result<(), InstallError> {
    match original {
        Some(contents) => atomic_write(path, contents),
        None => match fs::remove_file(path) {
            Ok(()) => {
                if let Some(parent) = path.parent() {
                    sync_directory(parent)?;
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        },
    }
}

pub(crate) fn remove_file(path: &Path) -> Result<(), InstallError> {
    match fs::remove_file(path) {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn ensure_private_directory(path: &Path) -> Result<(), InstallError> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), std::io::Error> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("integration plan is invalid or was not accepted")]
    InvalidPlan,
    #[error("configuration changed after the plan was created")]
    ConfigurationChanged,
    #[error("configuration could not be safely edited")]
    InvalidConfiguration,
    #[error("integration verification failed")]
    VerificationFailed,
    #[error("integration I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("integration JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::{build_coexistence_install_plan, build_install_plan, inspect_with_version};

    use super::*;

    static NEXT_TEST_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = NEXT_TEST_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "lili-integration-install-{}-{sequence}",
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
    fn install_preserves_unrelated_configuration_and_is_idempotent() {
        let temp = TempDir::new();
        let config = "# keep this comment\nmodel = \"gpt-5\"\n";
        let hooks = r#"{"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"existing-hook"}]}]},"future":true}"#;
        fs::write(temp.0.join(crate::CONFIG_FILE_NAME), config).unwrap();
        fs::write(temp.0.join(HOOKS_FILE_NAME), hooks).unwrap();
        let inspection = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
        let plan = build_install_plan(&inspection, &temp.0.join("bin/lili-hook"), 42);
        let outcome = install_with_verifier(&plan, |_| Ok(())).unwrap();
        assert!(outcome.changed);
        assert_eq!(outcome.backups.len(), 2);
        assert_eq!(fs::read_to_string(&outcome.backups[0]).unwrap(), config);
        let installed_config = fs::read_to_string(temp.0.join(crate::CONFIG_FILE_NAME)).unwrap();
        assert!(installed_config.contains("# keep this comment"));
        assert!(installed_config.contains("model = \"gpt-5\""));
        let installed_hooks = fs::read_to_string(temp.0.join(HOOKS_FILE_NAME)).unwrap();
        assert!(installed_hooks.contains("existing-hook"));
        assert!(installed_hooks.contains(LILI_INTEGRATION_ID));
        assert!(outcome.provenance.exists());

        let second_inspection = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
        let second_plan = build_install_plan(&second_inspection, &temp.0.join("bin/lili-hook"), 43);
        let second = install_with_verifier(&second_plan, |_| Ok(())).unwrap();
        assert!(!second.changed);
        assert!(second.backups.is_empty());
        let provenance: IntegrationProvenance =
            serde_json::from_slice(&fs::read(second.provenance).unwrap()).unwrap();
        assert_eq!(provenance.hook_commands.len(), 1);
    }

    #[test]
    fn verification_failure_rolls_back_every_configuration_file() {
        let temp = TempDir::new();
        let original = "model = \"gpt-5\"\n";
        fs::write(temp.0.join(crate::CONFIG_FILE_NAME), original).unwrap();
        let inspection = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
        let plan = build_install_plan(&inspection, &temp.0.join("bin/lili-hook"), 42);
        assert!(matches!(
            install_with_verifier(&plan, |_| Err(InstallError::VerificationFailed)),
            Err(InstallError::VerificationFailed)
        ));
        assert_eq!(
            fs::read_to_string(temp.0.join(crate::CONFIG_FILE_NAME)).unwrap(),
            original
        );
        assert!(!temp.0.join(HOOKS_FILE_NAME).exists());
        assert!(!provenance_path(&temp.0).exists());
    }

    #[test]
    fn changed_configuration_rejects_stale_plan_before_mutation() {
        let temp = TempDir::new();
        fs::write(temp.0.join(crate::CONFIG_FILE_NAME), "model = \"first\"\n").unwrap();
        let inspection = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
        let plan = build_install_plan(&inspection, &temp.0.join("bin/lili-hook"), 42);
        fs::write(temp.0.join(crate::CONFIG_FILE_NAME), "model = \"second\"\n").unwrap();
        assert!(matches!(
            install_with_verifier(&plan, |_| Ok(())),
            Err(InstallError::ConfigurationChanged)
        ));
        assert_eq!(
            fs::read_to_string(temp.0.join(crate::CONFIG_FILE_NAME)).unwrap(),
            "model = \"second\"\n"
        );
        assert!(!temp.0.join(HOOKS_FILE_NAME).exists());
    }

    #[test]
    fn coexistence_install_is_idempotent_and_records_original_argv() {
        let temp = TempDir::new();
        fs::write(
            temp.0.join(crate::CONFIG_FILE_NAME),
            "notify = [\"existing\", \"--channel\", \"pet\"]\n",
        )
        .unwrap();
        let hook_binary = temp.0.join("bin/lili-hook");
        let inspection = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
        let plan = build_coexistence_install_plan(&inspection, &hook_binary, 42);
        let first = install_with_verifier(&plan, |_| Ok(())).unwrap();
        let provenance: IntegrationProvenance =
            serde_json::from_slice(&fs::read(first.provenance).unwrap()).unwrap();
        assert_eq!(provenance.mode, IntegrationInstallMode::Coexist);
        assert_eq!(
            provenance.previous_notify_argv,
            Some(vec![
                "existing".to_owned(),
                "--channel".to_owned(),
                "pet".to_owned(),
            ])
        );

        let inspection = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
        let plan = build_coexistence_install_plan(&inspection, &hook_binary, 43);
        assert_eq!(plan.config_change.action, PlannedFileAction::Unchanged);
        let second = install_with_verifier(&plan, |_| Ok(())).unwrap();
        assert!(!second.changed);
    }
}
