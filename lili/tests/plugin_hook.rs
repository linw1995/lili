#![cfg(unix)]

use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Barrier,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use lili_session::{SessionEventKind, SpoolStore};

const FIXTURES: [&str; 5] = [
    include_str!("../../lili-session/tests/fixtures/codex/0.147.0/session-start.json"),
    include_str!("../../lili-session/tests/fixtures/codex/0.147.0/user-prompt-submit.json"),
    include_str!("../../lili-session/tests/fixtures/codex/0.147.0/permission-request.json"),
    include_str!("../../lili-session/tests/fixtures/codex/0.147.0/stop.json"),
    include_str!("../../lili-session/tests/fixtures/codex/0.147.0/session-end.json"),
];
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lili plugin hook test {} {sequence}",
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
fn packaged_launcher_forwards_concurrent_events_without_visible_output() {
    let Some(target) = supported_target() else {
        return;
    };
    let temp = TempDir::new();
    let plugin_root = temp.0.join("package with spaces");
    let launcher = install_plugin_runtime(&plugin_root, target);
    let codex_home = temp.0.join("codex home with spaces");
    let barrier = Arc::new(Barrier::new(FIXTURES.len() + 1));
    let started = Instant::now();
    let workers = FIXTURES.map(|fixture| {
        let barrier = barrier.clone();
        let launcher = launcher.clone();
        let plugin_root = plugin_root.clone();
        let codex_home = codex_home.clone();
        thread::spawn(move || {
            barrier.wait();
            invoke(&launcher, &plugin_root, &codex_home, fixture)
        })
    });
    barrier.wait();

    for worker in workers {
        let output = worker.join().unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert!(
            output.stdout.is_empty(),
            "plugin hooks must not emit model-visible output"
        );
        assert!(output.stderr.is_empty());
    }
    assert!(
        started.elapsed() <= Duration::from_secs(2),
        "concurrent plugin hooks exceeded their bounded execution budget"
    );

    let spool = SpoolStore::for_codex_home(&codex_home);
    let mut kinds = Vec::new();
    while let Some(claim) = spool.claim_next(unix_time_ms()).unwrap() {
        kinds.push(claim.event().event_type);
        claim.commit().unwrap();
    }
    assert_eq!(kinds.len(), FIXTURES.len());
    for expected in [
        SessionEventKind::SessionStarted,
        SessionEventKind::TurnStarted,
        SessionEventKind::AttentionRequired,
        SessionEventKind::TurnCompleted,
        SessionEventKind::SessionEnded,
    ] {
        assert!(
            kinds.contains(&expected),
            "missing spooled event: {expected:?}"
        );
    }
}

fn install_plugin_runtime(plugin_root: &Path, target: &str) -> PathBuf {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/lili");
    let launcher = plugin_root.join("hooks/forward");
    fs::create_dir_all(launcher.parent().unwrap()).unwrap();
    fs::copy(source_root.join("hooks/forward"), &launcher).unwrap();
    let mut launcher_permissions = fs::metadata(&launcher).unwrap().permissions();
    launcher_permissions.set_mode(0o755);
    fs::set_permissions(&launcher, launcher_permissions).unwrap();

    let forwarder = plugin_root.join("bin").join(target).join("lili-hook");
    fs::create_dir_all(forwarder.parent().unwrap()).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_lili-hook"), &forwarder).unwrap();
    let mut forwarder_permissions = fs::metadata(&forwarder).unwrap().permissions();
    forwarder_permissions.set_mode(0o755);
    fs::set_permissions(&forwarder, forwarder_permissions).unwrap();
    launcher
}

fn invoke(
    launcher: &Path,
    plugin_root: &Path,
    codex_home: &Path,
    fixture: &str,
) -> std::process::Output {
    let mut child = Command::new(launcher)
        .env("PLUGIN_ROOT", plugin_root)
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
        .write_all(fixture.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn supported_target() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("arm64-apple-darwin")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("x86_64-unknown-linux-gnu")
    } else {
        None
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}
