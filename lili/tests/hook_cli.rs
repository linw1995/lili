use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("lili-hook-cli-{}-{sequence}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn payload() -> &'static str {
    r#"{"version":1,"provider":"codex","type":"attention_required","eventId":"event-1","sessionId":"session-1","turnId":"turn-1","occurredAtMs":10}"#
}

#[test]
fn version_matches_the_workspace_release() {
    let output = Command::new(env!("CARGO_BIN_EXE_lili-hook"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("lili-hook {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn argv_mode_spools_without_emitting_approval_output() {
    let temp = TempDir::new();
    let output = Command::new(env!("CARGO_BIN_EXE_lili-hook"))
        .args(["--json-argv", payload()])
        .env("CODEX_HOME", &temp.0)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn stdin_mode_has_deterministic_invalid_input_exit_code() {
    let temp = TempDir::new();
    let mut child = Command::new(env!("CARGO_BIN_EXE_lili-hook"))
        .arg("--json-stdin")
        .env("CODEX_HOME", &temp.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"not-json").unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}
