use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use lili_integration::{
    IntegrationInspection, PluginLifecycleHost, PluginMigrationError, build_install_plan,
    inspect_with_version, install_with_verifier, remove_plugin_with_host,
};
use lili_session::TESTED_CODEX_VERSION;

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lili-plugin-removal-{}-{sequence}",
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

#[derive(Default)]
struct RemovalHost {
    remove_calls: usize,
}

impl PluginLifecycleHost for RemovalHost {
    fn install(
        &mut self,
        _codex_home: &Path,
        _plugin_selector: &str,
    ) -> Result<(), PluginMigrationError> {
        panic!("plugin removal must not install")
    }

    fn inspect(&mut self, _codex_home: &Path, _plugin_selector: &str) -> IntegrationInspection {
        panic!("plugin removal must not inspect or mutate Lili state")
    }

    fn hooks_trusted(&mut self, _codex_home: &Path, _plugin_selector: &str) -> bool {
        panic!("plugin removal must not inspect hook trust")
    }

    fn rollback(
        &mut self,
        _codex_home: &Path,
        plugin_selector: &str,
    ) -> Result<(), PluginMigrationError> {
        assert_eq!(plugin_selector, "lili@test-marketplace");
        self.remove_calls += 1;
        Ok(())
    }
}

#[test]
fn plugin_removal_preserves_desktop_data_and_all_codex_configuration() {
    let temp = TempDir::new();
    let protected = [
        (
            "applications/Lili.app/Contents/Info.plist",
            b"desktop".as_slice(),
        ),
        ("pets/lili/pet.json", b"pet".as_slice()),
        ("lili/actions.toml", b"actions".as_slice()),
        ("lili/state.json", b"state".as_slice()),
        ("lili/spool/event.json", b"spool".as_slice()),
    ];
    for (relative, contents) in protected {
        let path = temp.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    fs::write(
        temp.0.join("config.toml"),
        "model = \"gpt-5\"\nnotify = [\"unrelated\"]\n",
    )
    .unwrap();
    fs::write(
        temp.0.join("hooks.json"),
        r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"unrelated-hook"}]}]}}"#,
    )
    .unwrap();
    let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
    let plan = lili_integration::build_coexistence_install_plan(
        &inspection,
        &temp.0.join("bin/lili-hook"),
        42,
    );
    install_with_verifier(&plan, |_| Ok(())).unwrap();

    let paths = [
        "applications/Lili.app/Contents/Info.plist",
        "pets/lili/pet.json",
        "lili/actions.toml",
        "lili/state.json",
        "lili/spool/event.json",
        "lili/integration.json",
        "config.toml",
        "hooks.json",
    ];
    let before = paths.map(|relative| (relative, fs::read(temp.0.join(relative)).unwrap()));

    let mut host = RemovalHost::default();
    let outcome = remove_plugin_with_host(&mut host, &temp.0, "lili@test-marketplace").unwrap();
    assert_eq!(host.remove_calls, 1);
    assert!(!outcome.legacy_configuration_changed);
    assert!(!outcome.desktop_application_changed);
    assert!(!outcome.application_data_changed);
    for (relative, contents) in before {
        assert_eq!(fs::read(temp.0.join(relative)).unwrap(), contents);
    }
}

#[test]
fn plugin_rollback_does_not_restore_or_create_legacy_configuration() {
    let temp = TempDir::new();
    let inspection = inspect_with_version(&temp.0, Some(TESTED_CODEX_VERSION.to_owned()));
    let plan = build_install_plan(&inspection, &temp.0.join("bin/lili-hook"), 42);
    assert_eq!(plan.codex_home, temp.0);

    let mut host = RemovalHost::default();
    remove_plugin_with_host(&mut host, &temp.0, "lili@test-marketplace").unwrap();
    assert_eq!(host.remove_calls, 1);
    assert!(!temp.0.join("config.toml").exists());
    assert!(!temp.0.join("hooks.json").exists());
    assert!(!temp.0.join("lili/integration.json").exists());
}
