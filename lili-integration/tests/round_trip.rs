use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use lili_integration::{
    CONFIG_FILE_NAME, HOOKS_FILE_NAME, InstallError, InstallPlanStatus, IntegrationKind,
    LILI_INTEGRATION_ID, UninstallError, build_install_plan, inspect_with_version,
    install_with_verifier, uninstall,
};
use lili_session::CodexIntegrationSurface;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lili-integration-round-trip-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn hook_binary(&self) -> PathBuf {
        self.0.join("bin/lili-hook")
    }

    fn install(&self, timestamp: u64) {
        let inspection = inspect_with_version(&self.0, Some("0.147.0".to_owned()));
        let plan = build_install_plan(&inspection, &self.hook_binary(), timestamp);
        install_with_verifier(&plan, |_| Ok(())).unwrap();
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn empty_configuration_round_trips_to_absence() {
    let temp = TempDir::new();
    temp.install(1);
    let installed = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
    assert_eq!(installed.notify.kind, IntegrationKind::Lili);
    assert!(
        installed
            .hook_surfaces
            .iter()
            .all(|surface| surface.lili_handlers == 1)
    );
    assert!(uninstall(&temp.0).unwrap().complete);
    assert!(!temp.0.join(CONFIG_FILE_NAME).exists());
    assert!(!temp.0.join(HOOKS_FILE_NAME).exists());
}

#[test]
fn commented_configuration_round_trip_preserves_comments() {
    let temp = TempDir::new();
    let original = "# model selection\nmodel = \"gpt-5\" # keep inline\n";
    fs::write(temp.0.join(CONFIG_FILE_NAME), original).unwrap();
    temp.install(2);
    assert!(
        fs::read_to_string(temp.0.join(CONFIG_FILE_NAME))
            .unwrap()
            .contains("# keep inline")
    );
    assert!(uninstall(&temp.0).unwrap().complete);
    assert_eq!(
        fs::read_to_string(temp.0.join(CONFIG_FILE_NAME)).unwrap(),
        original
    );
}

#[test]
fn reordered_json_round_trip_preserves_unrelated_structure() {
    let temp = TempDir::new();
    fs::write(
        temp.0.join(HOOKS_FILE_NAME),
        r#"{"zFuture":{"enabled":true},"hooks":{},"aFuture":[1,2]}"#,
    )
    .unwrap();
    temp.install(3);
    assert!(uninstall(&temp.0).unwrap().complete);
    let document: serde_json::Value =
        serde_json::from_slice(&fs::read(temp.0.join(HOOKS_FILE_NAME)).unwrap()).unwrap();
    assert_eq!(document["zFuture"]["enabled"], true);
    assert_eq!(document["aFuture"], serde_json::json!([1, 2]));
    assert_eq!(document["hooks"], serde_json::json!({}));
}

#[test]
fn existing_hook_survives_install_repeat_and_uninstall() {
    let temp = TempDir::new();
    fs::write(
        temp.0.join(HOOKS_FILE_NAME),
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"existing-hook --flag"}]}]}}"#,
    )
    .unwrap();
    temp.install(4);
    temp.install(5);
    let installed = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
    let session_start = installed
        .hook_surfaces
        .iter()
        .find(|surface| surface.surface == CodexIntegrationSurface::SessionStart)
        .unwrap();
    assert_eq!(session_start.lili_handlers, 1);
    assert_eq!(session_start.other_handlers, 1);
    assert!(uninstall(&temp.0).unwrap().complete);
    let hooks = fs::read_to_string(temp.0.join(HOOKS_FILE_NAME)).unwrap();
    assert!(hooks.contains("existing-hook --flag"));
    assert!(!hooks.contains(LILI_INTEGRATION_ID));
}

#[test]
fn conflicting_notify_is_rejected_without_mutation() {
    let temp = TempDir::new();
    let original = "notify = [\"existing\", \"private-argument\"]\n";
    fs::write(temp.0.join(CONFIG_FILE_NAME), original).unwrap();
    let inspection = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
    let plan = build_install_plan(&inspection, &temp.hook_binary(), 6);
    assert_eq!(plan.status, InstallPlanStatus::Conflict);
    assert!(matches!(
        install_with_verifier(&plan, |_| Ok(())),
        Err(InstallError::InvalidPlan)
    ));
    assert_eq!(
        fs::read_to_string(temp.0.join(CONFIG_FILE_NAME)).unwrap(),
        original
    );
    assert!(!temp.0.join(HOOKS_FILE_NAME).exists());
}

#[test]
fn repeated_install_has_one_handler_per_surface() {
    let temp = TempDir::new();
    temp.install(7);
    temp.install(8);
    let inspection = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
    assert!(
        inspection
            .hook_surfaces
            .iter()
            .all(|surface| surface.lili_handlers == 1)
    );
    assert!(uninstall(&temp.0).unwrap().complete);
}

#[test]
fn modified_after_install_keeps_user_changes_during_uninstall() {
    let temp = TempDir::new();
    fs::write(temp.0.join(CONFIG_FILE_NAME), "model = \"gpt-5\"\n").unwrap();
    temp.install(9);
    let modified = "# added later\nmodel = \"gpt-5.1\"\nnotify = [\"replacement\"]\n";
    fs::write(temp.0.join(CONFIG_FILE_NAME), modified).unwrap();
    let outcome = uninstall(&temp.0).unwrap();
    assert!(outcome.complete);
    assert_eq!(outcome.conflicts.len(), 1);
    assert_eq!(
        fs::read_to_string(temp.0.join(CONFIG_FILE_NAME)).unwrap(),
        modified
    );
}

#[test]
fn uninstall_removes_only_owned_entries() {
    let temp = TempDir::new();
    fs::write(temp.0.join(CONFIG_FILE_NAME), "model = \"gpt-5\"\n").unwrap();
    fs::write(
        temp.0.join(HOOKS_FILE_NAME),
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"audit-stop"}]}]},"future":true}"#,
    )
    .unwrap();
    fs::create_dir_all(temp.0.join("pet/custom")).unwrap();
    fs::write(temp.0.join("pet/custom/pet.json"), "{}\n").unwrap();
    temp.install(10);
    let outcome = uninstall(&temp.0).unwrap();
    assert!(outcome.complete);
    assert!(
        fs::read_to_string(temp.0.join(CONFIG_FILE_NAME))
            .unwrap()
            .contains("model = \"gpt-5\"")
    );
    let hooks = fs::read_to_string(temp.0.join(HOOKS_FILE_NAME)).unwrap();
    assert!(hooks.contains("audit-stop"));
    assert!(hooks.contains("\"future\": true"));
    assert!(temp.0.join("pet/custom/pet.json").exists());
}

#[test]
fn corrupted_configuration_fails_closed_at_each_boundary() {
    let temp = TempDir::new();
    fs::write(temp.0.join(CONFIG_FILE_NAME), "notify = [").unwrap();
    let inspection = inspect_with_version(&temp.0, Some("0.147.0".to_owned()));
    let blocked = build_install_plan(&inspection, &temp.hook_binary(), 11);
    assert_eq!(blocked.status, InstallPlanStatus::Blocked);
    assert!(matches!(
        install_with_verifier(&blocked, |_| Ok(())),
        Err(InstallError::InvalidPlan)
    ));

    fs::write(temp.0.join(CONFIG_FILE_NAME), "model = \"gpt-5\"\n").unwrap();
    temp.install(12);
    fs::write(temp.0.join(HOOKS_FILE_NAME), b"{").unwrap();
    assert!(matches!(
        uninstall(&temp.0),
        Err(UninstallError::InvalidConfiguration)
    ));
    assert!(temp.0.join("lili/integration.json").exists());
}
