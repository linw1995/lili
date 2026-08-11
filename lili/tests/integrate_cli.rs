use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lili-integrate-cli-{}-{sequence}",
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
fn inspect_cli_reports_safe_effective_configuration() {
    let temp = TempDir::new();
    fs::write(
        temp.0.join("config.toml"),
        "api_key = \"never-print-this\"\nnotify = [\"existing\", \"private-argument\"]\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lili"))
        .args(["integrate", "inspect"])
        .env("CODEX_HOME", &temp.0)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let inspection: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(inspection["codexHome"], temp.0.to_string_lossy().as_ref());
    assert_eq!(inspection["notify"]["kind"], "other");
    let output = String::from_utf8(output.stdout).unwrap();
    assert!(!output.contains("never-print-this"));
    assert!(!output.contains("private-argument"));
}

#[test]
fn plan_cli_reports_exact_changes_without_mutating_configuration() {
    let temp = TempDir::new();
    let output = Command::new(env!("CARGO_BIN_EXE_lili"))
        .args(["integrate", "plan"])
        .env("CODEX_HOME", &temp.0)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(plan["status"], "ready");
    assert_eq!(plan["configChange"]["action"], "create");
    assert_eq!(plan["hooksChange"]["action"], "create");
    assert_eq!(plan["hookAdditions"].as_array().unwrap().len(), 5);
    assert_eq!(
        plan["notify"]["argv"][2],
        lili_integration::LILI_INTEGRATION_ID
    );
    assert!(!temp.0.join("config.toml").exists());
    assert!(!temp.0.join("hooks.json").exists());
}

#[test]
fn coexistence_plan_requires_explicit_mode_and_preserves_argv() {
    let temp = TempDir::new();
    fs::write(
        temp.0.join("config.toml"),
        "notify = [\"existing\", \"--channel\", \"pet\"]\n",
    )
    .unwrap();
    let default = Command::new(env!("CARGO_BIN_EXE_lili"))
        .args(["integrate", "plan"])
        .env("CODEX_HOME", &temp.0)
        .output()
        .unwrap();
    let default: serde_json::Value = serde_json::from_slice(&default.stdout).unwrap();
    assert_eq!(default["status"], "conflict");

    let coexist = Command::new(env!("CARGO_BIN_EXE_lili"))
        .args(["integrate", "plan", "--coexist"])
        .env("CODEX_HOME", &temp.0)
        .output()
        .unwrap();
    assert_eq!(coexist.status.code(), Some(0));
    let coexist: serde_json::Value = serde_json::from_slice(&coexist.stdout).unwrap();
    assert_eq!(coexist["status"], "ready");
    assert_eq!(coexist["mode"], "coexist");
    assert_eq!(coexist["notify"]["argv"][3], "--coexist-notify-json");
    assert_eq!(
        coexist["previousNotifyArgv"],
        serde_json::json!(["existing", "--channel", "pet"])
    );
}
