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
