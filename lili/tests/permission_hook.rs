#![cfg(unix)]

use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use lili_lib::hook_forwarder::UNRESPONSIVE_ENDPOINT_BUDGET;
use lili_session::{BoundForwardingEndpoint, SessionEventKind, SpoolStore};

const PERMISSION_FIXTURE: &str =
    include_str!("../../lili-session/tests/fixtures/codex/0.147.0/permission-request.json");
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lili-permission-hook-{}-{sequence}",
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
fn stopped_application_spools_permission_without_decision_output() {
    let temp = TempDir::new();
    let started = Instant::now();
    let output = run_permission_hook(&temp.0);
    assert_observer_only_success(&output, started.elapsed());
    assert_one_permission_event(&temp.0);
}

#[tokio::test]
async fn restarting_application_ignores_stale_instance_without_decision_output() {
    let temp = TempDir::new();
    let runtime_dir = temp.0.join("lili").join("runtime");
    let endpoint = BoundForwardingEndpoint::bind(&runtime_dir).unwrap();
    let credential_store = endpoint.credential_store().clone();
    let stale_record = credential_store.load().unwrap();
    drop(endpoint);

    let mut options = OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    let mut file = options.open(credential_store.path()).unwrap();
    serde_json::to_writer(&mut file, &stale_record).unwrap();
    file.write_all(b"\n").unwrap();
    file.sync_all().unwrap();

    let started = Instant::now();
    let output = run_permission_hook(&temp.0);
    assert_observer_only_success(&output, started.elapsed());
    assert_one_permission_event(&temp.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hung_application_falls_back_without_decision_output() {
    let temp = TempDir::new();
    let runtime_dir = temp.0.join("lili").join("runtime");
    let endpoint = BoundForwardingEndpoint::bind(&runtime_dir).unwrap();
    let server = tokio::spawn(async move {
        let mut connection = endpoint.accept().await.unwrap();
        connection.read_payload().await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
    });

    let started = Instant::now();
    let codex_home = temp.0.clone();
    let output = tokio::task::spawn_blocking(move || run_permission_hook(&codex_home))
        .await
        .unwrap();
    let elapsed = started.elapsed();
    server.await.unwrap();
    assert_observer_only_success(&output, elapsed);
    assert_one_permission_event(&temp.0);
}

fn run_permission_hook(codex_home: &Path) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lili-hook"))
        .arg("--json-stdin")
        .env("CODEX_HOME", codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(PERMISSION_FIXTURE.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn assert_observer_only_success(output: &std::process::Output, elapsed: Duration) {
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty(), "permission hooks must not decide");
    assert!(output.stderr.is_empty());
    assert!(
        elapsed <= UNRESPONSIVE_ENDPOINT_BUDGET,
        "permission forwarding exceeded its bounded fallback budget"
    );
}

fn assert_one_permission_event(codex_home: &Path) {
    let spool = SpoolStore::for_codex_home(codex_home);
    let claim = spool.claim_next(unix_time_ms()).unwrap().unwrap();
    assert_eq!(
        claim.event().event_type,
        SessionEventKind::AttentionRequired
    );
    claim.commit().unwrap();
    assert!(spool.claim_next(unix_time_ms()).unwrap().is_none());
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}
