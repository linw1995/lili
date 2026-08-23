#![cfg(unix)]

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use lili_lib::hook_forwarder::UNRESPONSIVE_ENDPOINT_BUDGET;
use lili_session::{BoundForwardingEndpoint, SessionEventKind, SqliteSpoolStore};
use lili_storage::ApplicationPaths;
use tokio::sync::Mutex;

const PERMISSION_FIXTURE: &str =
    include_str!("../../lili-session/tests/fixtures/codex/0.147.0/permission-request.json");
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
static HOOK_TEST_LOCK: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(format!(
            "/tmp/lili-permission-hook-{}-{sequence}",
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
    let _lock = HOOK_TEST_LOCK.blocking_lock();
    let temp = TempDir::new();
    let home = short_home();
    let started = Instant::now();
    let output = run_permission_hook(&home, &temp.0.join("codex-home"));
    assert_observer_only_success(&output, started.elapsed());
    assert_one_permission_event(&application_paths(&home));
    fs::remove_dir_all(home).unwrap();
}

#[tokio::test]
async fn restarting_application_ignores_stale_instance_without_decision_output() {
    let _lock = HOOK_TEST_LOCK.lock().await;
    let temp = TempDir::new();
    let home = short_home();
    let paths = application_paths(&home);
    let runtime_dir = paths.runtime_root();
    let endpoint = BoundForwardingEndpoint::bind(&runtime_dir).unwrap();
    let credential_store = endpoint.credential_store().clone();
    let stale_instance = endpoint.credentials().instance_id().to_owned();
    drop(endpoint);
    assert_eq!(
        credential_store.load().unwrap().instance_id(),
        stale_instance
    );

    let started = Instant::now();
    let output = run_permission_hook(&home, &temp.0.join("codex-home"));
    assert_observer_only_success(&output, started.elapsed());
    assert_one_permission_event(&paths);
    fs::remove_dir_all(home).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hung_application_falls_back_without_decision_output() {
    let _lock = HOOK_TEST_LOCK.lock().await;
    let temp = TempDir::new();
    let home = short_home();
    let paths = application_paths(&home);
    let runtime_dir = paths.runtime_root();
    let endpoint = BoundForwardingEndpoint::bind(&runtime_dir).unwrap();
    let server = tokio::spawn(async move {
        let mut connection = endpoint.accept().await.unwrap();
        connection.read_payload().await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
    });

    let started = Instant::now();
    let codex_home = temp.0.join("codex-home");
    let hook_home = home.clone();
    let output = tokio::task::spawn_blocking(move || run_permission_hook(&hook_home, &codex_home))
        .await
        .unwrap();
    let elapsed = started.elapsed();
    server.await.unwrap();
    assert_observer_only_success(&output, elapsed);
    assert_one_permission_event(&paths);
    fs::remove_dir_all(home).unwrap();
}

fn run_permission_hook(home: &Path, codex_home: &Path) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lili-hook"))
        .arg("--json-stdin")
        .env("CODEX_HOME", codex_home)
        .env("HOME", home)
        .env("XDG_STATE_HOME", home.join("state"))
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

fn assert_one_permission_event(paths: &ApplicationPaths) {
    let spool = SqliteSpoolStore::for_application(paths.clone());
    let claim = spool.claim_next(unix_time_ms()).unwrap().unwrap();
    assert_eq!(
        claim.event().event_type,
        SessionEventKind::AttentionRequired
    );
    claim.commit().unwrap();
    assert!(spool.claim_next(unix_time_ms()).unwrap().is_none());
}

fn application_paths(home: &Path) -> ApplicationPaths {
    #[cfg(target_os = "macos")]
    let root = home
        .join("Library")
        .join("Application Support")
        .join(lili_storage::APPLICATION_IDENTIFIER);
    #[cfg(target_os = "linux")]
    let root = home
        .join("state")
        .join(lili_storage::APPLICATION_IDENTIFIER);
    ApplicationPaths::from_root(root).unwrap()
}

fn short_home() -> PathBuf {
    let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let home = PathBuf::from(format!(
        "/tmp/lili-permission-home-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&home).unwrap();
    home
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}
