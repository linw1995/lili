use std::path::{Path, PathBuf};

use lili_session::CodexIntegrationSurface;
use serde::{Deserialize, Serialize};

use crate::{
    IntegrationFileInspection, IntegrationFileStatus, IntegrationInspection, IntegrationKind,
    LILI_INTEGRATION_ID,
};

const HOOK_SURFACES: [(CodexIntegrationSurface, &str); 5] = [
    (CodexIntegrationSurface::SessionStart, "SessionStart"),
    (
        CodexIntegrationSurface::UserPromptSubmit,
        "UserPromptSubmit",
    ),
    (
        CodexIntegrationSurface::PermissionRequest,
        "PermissionRequest",
    ),
    (CodexIntegrationSurface::Stop, "Stop"),
    (CodexIntegrationSurface::SessionEnd, "SessionEnd"),
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallPlanStatus {
    Ready,
    Conflict,
    Blocked,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationInstallMode {
    Exclusive,
    Coexist,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannedFileAction {
    Create,
    Update,
    Unchanged,
    Conflict,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedNotifyEntry {
    pub argv: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedHookEntry {
    pub surface: CodexIntegrationSurface,
    pub event_name: String,
    pub handler_type: String,
    pub command: String,
    pub command_windows: String,
    pub timeout_seconds: u64,
    pub integration_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedFileChange {
    pub target: PathBuf,
    pub expected_content_sha256: Option<String>,
    pub action: PlannedFileAction,
    pub backup: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationInstallPlan {
    pub schema_version: u16,
    pub mode: IntegrationInstallMode,
    pub status: InstallPlanStatus,
    pub codex_home: PathBuf,
    pub codex_version: Option<String>,
    pub hook_binary: PathBuf,
    pub config_change: PlannedFileChange,
    pub hooks_change: PlannedFileChange,
    pub notify: PlannedNotifyEntry,
    pub previous_notify_argv: Option<Vec<String>>,
    pub hook_additions: Vec<PlannedHookEntry>,
    pub trust_requirements: Vec<String>,
    pub verification_command: Vec<String>,
    pub blocking_reasons: Vec<String>,
}

pub fn build_install_plan(
    inspection: &IntegrationInspection,
    hook_binary: &Path,
    timestamp_ms: u64,
) -> IntegrationInstallPlan {
    build_plan(
        inspection,
        hook_binary,
        timestamp_ms,
        IntegrationInstallMode::Exclusive,
    )
}

pub fn build_coexistence_install_plan(
    inspection: &IntegrationInspection,
    hook_binary: &Path,
    timestamp_ms: u64,
) -> IntegrationInstallPlan {
    build_plan(
        inspection,
        hook_binary,
        timestamp_ms,
        IntegrationInstallMode::Coexist,
    )
}

fn build_plan(
    inspection: &IntegrationInspection,
    hook_binary: &Path,
    timestamp_ms: u64,
    mode: IntegrationInstallMode,
) -> IntegrationInstallPlan {
    let previous_notify_argv = coexistence_original_argv(inspection, mode);
    let notify = PlannedNotifyEntry {
        argv: notify_argv(hook_binary, previous_notify_argv.as_deref()),
    };
    let hook_additions = HOOK_SURFACES
        .into_iter()
        .filter(|(surface, _)| {
            inspection
                .hook_surfaces
                .iter()
                .find(|existing| existing.surface == *surface)
                .is_none_or(|existing| existing.lili_handlers == 0)
        })
        .map(|(surface, event_name)| planned_hook(surface, event_name, hook_binary))
        .collect::<Vec<_>>();
    let mut blocking_reasons = Vec::new();
    if !hook_binary.is_absolute() {
        blocking_reasons.push("The packaged hook binary path is not absolute.".to_owned());
    }
    if !safe_to_mutate(&inspection.config) || !safe_to_mutate(&inspection.hooks) {
        blocking_reasons.push(
            "Configuration is malformed, unreadable, oversized, or symbolic-linked.".to_owned(),
        );
    }
    let status = if !blocking_reasons.is_empty() {
        InstallPlanStatus::Blocked
    } else if inspection.notify.kind == IntegrationKind::Invalid {
        blocking_reasons
            .push("The existing notify value is invalid and cannot be preserved.".to_owned());
        InstallPlanStatus::Conflict
    } else if inspection.notify.kind == IntegrationKind::Other
        && mode == IntegrationInstallMode::Exclusive
    {
        blocking_reasons
            .push("An existing non-Lili notify command requires explicit coexistence.".to_owned());
        InstallPlanStatus::Conflict
    } else {
        InstallPlanStatus::Ready
    };
    let config_action = match status {
        InstallPlanStatus::Blocked => PlannedFileAction::Blocked,
        InstallPlanStatus::Conflict => PlannedFileAction::Conflict,
        InstallPlanStatus::Ready
            if inspection.notify.argv.as_deref() == Some(notify.argv.as_slice()) =>
        {
            PlannedFileAction::Unchanged
        }
        InstallPlanStatus::Ready => action_for(&inspection.config),
    };
    let hooks_action = match status {
        InstallPlanStatus::Blocked => PlannedFileAction::Blocked,
        _ if hook_additions.is_empty() => PlannedFileAction::Unchanged,
        _ => action_for(&inspection.hooks),
    };
    let config_change = planned_file_change(&inspection.config, config_action, timestamp_ms);
    let hooks_change = planned_file_change(&inspection.hooks, hooks_action, timestamp_ms);
    let verification_payload = serde_json::json!({
        "type": "agent-turn-complete",
        "thread-id": "lili-verification",
        "turn-id": timestamp_ms.to_string(),
        "cwd": inspection.codex_home,
        "client": "lili-integrator",
        "last-assistant-message": "Lili integration verified."
    })
    .to_string();

    IntegrationInstallPlan {
        schema_version: 1,
        mode,
        status,
        codex_home: inspection.codex_home.clone(),
        codex_version: inspection.codex_version.clone(),
        hook_binary: hook_binary.to_path_buf(),
        config_change,
        hooks_change,
        notify,
        previous_notify_argv,
        hook_additions,
        trust_requirements: vec![
            "Review the absolute lili-hook path before applying this plan.".to_owned(),
            "Codex may require explicit trust for newly configured user hooks.".to_owned(),
            "PermissionRequest remains observer-only and returns no decision.".to_owned(),
        ],
        verification_command: vec![
            hook_binary.display().to_string(),
            "--integration-id".to_owned(),
            LILI_INTEGRATION_ID.to_owned(),
            "--json-argv".to_owned(),
            verification_payload,
        ],
        blocking_reasons,
    }
}

fn notify_argv(hook_binary: &Path, previous: Option<&[String]>) -> Vec<String> {
    let mut argv = vec![
        hook_binary.display().to_string(),
        "--integration-id".to_owned(),
        LILI_INTEGRATION_ID.to_owned(),
    ];
    match previous {
        Some(previous) => {
            argv.push("--coexist-notify-json".to_owned());
            argv.push(serde_json::to_string(previous).expect("notify argv must serialize"));
        }
        None => argv.push("--json-argv".to_owned()),
    }
    argv
}

fn coexistence_original_argv(
    inspection: &IntegrationInspection,
    mode: IntegrationInstallMode,
) -> Option<Vec<String>> {
    if mode != IntegrationInstallMode::Coexist {
        return None;
    }
    if inspection.notify.kind == IntegrationKind::Other {
        return inspection.notify.argv.clone();
    }
    let argv = inspection.notify.argv.as_deref()?;
    if argv.get(1).map(String::as_str) != Some("--integration-id")
        || argv.get(2).map(String::as_str) != Some(LILI_INTEGRATION_ID)
        || argv.get(3).map(String::as_str) != Some("--coexist-notify-json")
        || argv.len() != 5
    {
        return None;
    }
    serde_json::from_str(&argv[4]).ok()
}

fn planned_hook(
    surface: CodexIntegrationSurface,
    event_name: &str,
    hook_binary: &Path,
) -> PlannedHookEntry {
    let suffix = format!("--integration-id {LILI_INTEGRATION_ID} --json-stdin");
    PlannedHookEntry {
        surface,
        event_name: event_name.to_owned(),
        handler_type: "command".to_owned(),
        command: format!("{} {suffix}", quote_posix(hook_binary)),
        command_windows: format!("{} {suffix}", quote_windows(hook_binary)),
        timeout_seconds: 1,
        integration_id: LILI_INTEGRATION_ID.to_owned(),
    }
}

fn safe_to_mutate(file: &IntegrationFileInspection) -> bool {
    !file.symbolic_link
        && matches!(
            file.status,
            IntegrationFileStatus::Missing | IntegrationFileStatus::Parsed
        )
}

fn action_for(file: &IntegrationFileInspection) -> PlannedFileAction {
    if file.status == IntegrationFileStatus::Missing {
        PlannedFileAction::Create
    } else {
        PlannedFileAction::Update
    }
}

fn planned_file_change(
    file: &IntegrationFileInspection,
    action: PlannedFileAction,
    timestamp_ms: u64,
) -> PlannedFileChange {
    let backup = (action == PlannedFileAction::Update).then(|| {
        let file_name = file
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("configuration");
        file.path
            .with_file_name(format!("{file_name}.lili-backup-{timestamp_ms}"))
    });
    PlannedFileChange {
        target: file.path.clone(),
        expected_content_sha256: file.content_sha256.clone(),
        action,
        backup,
    }
}

fn quote_posix(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\"'\"'"))
}

fn quote_windows(path: &Path) -> String {
    format!("\"{}\"", path.display().to_string().replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::{IntegrationKind, inspect_with_version};

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "lili-integration-plan-{}-{sequence}",
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
    fn clean_plan_is_exact_and_deterministic_without_mutation() {
        let temp = TempDir::new();
        let hook_binary = temp.0.join("bin/lili-hook");
        let inspection = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
        let first = build_install_plan(&inspection, &hook_binary, 42);
        let second = build_install_plan(&inspection, &hook_binary, 42);
        assert_eq!(first, second);
        assert_eq!(first.status, InstallPlanStatus::Ready);
        assert_eq!(first.config_change.action, PlannedFileAction::Create);
        assert_eq!(first.hooks_change.action, PlannedFileAction::Create);
        assert_eq!(first.hook_additions.len(), 5);
        assert_eq!(first.notify.argv[2], LILI_INTEGRATION_ID);
        assert!(first.config_change.backup.is_none());
        assert!(!temp.0.join("config.toml").exists());
        assert!(!temp.0.join("hooks.json").exists());
    }

    #[test]
    fn existing_notify_is_a_hard_conflict_in_the_default_plan() {
        let temp = TempDir::new();
        fs::write(temp.0.join("config.toml"), "notify = [\"existing\"]\n").unwrap();
        let inspection = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
        assert_eq!(inspection.notify.kind, IntegrationKind::Other);
        let plan = build_install_plan(&inspection, &temp.0.join("lili-hook"), 42);
        assert_eq!(plan.status, InstallPlanStatus::Conflict);
        assert_eq!(plan.config_change.action, PlannedFileAction::Conflict);
        assert_eq!(
            fs::read_to_string(temp.0.join("config.toml")).unwrap(),
            "notify = [\"existing\"]\n"
        );
    }

    #[test]
    fn explicit_coexistence_wraps_the_original_notify_argv() {
        let temp = TempDir::new();
        fs::write(
            temp.0.join("config.toml"),
            "notify = [\"existing\", \"--channel\", \"pet\"]\n",
        )
        .unwrap();
        let hook_binary = temp.0.join("lili-hook");
        let inspection = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
        let plan = build_coexistence_install_plan(&inspection, &hook_binary, 42);
        assert_eq!(plan.status, InstallPlanStatus::Ready);
        assert_eq!(plan.mode, IntegrationInstallMode::Coexist);
        assert_eq!(
            plan.previous_notify_argv,
            Some(vec![
                "existing".to_owned(),
                "--channel".to_owned(),
                "pet".to_owned(),
            ])
        );
        assert_eq!(plan.notify.argv[3], "--coexist-notify-json");
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&plan.notify.argv[4]).unwrap(),
            plan.previous_notify_argv.unwrap()
        );
    }
}
