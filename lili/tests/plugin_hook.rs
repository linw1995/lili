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

use lili_session::{ReductionOutcome, SessionEventKind, SessionReducer, SqliteSpoolStore};
use lili_storage::ApplicationPaths;

const FIXTURES: [&str; 5] = [
    include_str!("../../lili-session/tests/fixtures/codex/0.147.0/session-start.json"),
    include_str!("../../lili-session/tests/fixtures/codex/0.147.0/user-prompt-submit.json"),
    include_str!("../../lili-session/tests/fixtures/codex/0.147.0/permission-request.json"),
    include_str!("../../lili-session/tests/fixtures/codex/0.147.0/stop.json"),
    include_str!("../../lili-session/tests/fixtures/codex/0.147.0/session-end.json"),
];
const CONCURRENT_HOOK_TEST_BUDGET: Duration = Duration::from_secs(5);
const VERSIONED_FIXTURES: [(&str, &str); 5] = [
    (
        "SessionStart",
        include_str!("../../lili-session/tests/fixtures/codex/0.147.0/session-start.json"),
    ),
    (
        "UserPromptSubmit",
        include_str!("../../lili-session/tests/fixtures/codex/0.147.0/user-prompt-submit.json"),
    ),
    (
        "PermissionRequest",
        include_str!("../../lili-session/tests/fixtures/codex/0.147.0/permission-request.json"),
    ),
    (
        "Stop",
        include_str!("../../lili-session/tests/fixtures/codex/0.147.0/stop.json"),
    ),
    (
        "SessionEnd",
        include_str!("../../lili-session/tests/fixtures/codex/0.147.0/session-end.json"),
    ),
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
    let home = temp.0.join("home");
    let codex_home = temp.0.join("codex home with spaces");
    let plugin_root = codex_home
        .join("plugins/cache/lili-local/lili")
        .join(env!("CARGO_PKG_VERSION"));
    let launcher = install_plugin_runtime(&plugin_root, target);
    let barrier = Arc::new(Barrier::new(FIXTURES.len() + 1));
    let started = Instant::now();
    let workers = FIXTURES.map(|fixture| {
        let barrier = barrier.clone();
        let launcher = launcher.clone();
        let plugin_root = plugin_root.clone();
        let codex_home = codex_home.clone();
        let home = home.clone();
        thread::spawn(move || {
            barrier.wait();
            invoke(&launcher, &plugin_root, &codex_home, &home, fixture)
        })
    });
    barrier.wait();

    for worker in workers {
        let output = worker.join().unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "plugin hook failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout.is_empty(),
            "plugin hooks must not emit model-visible output"
        );
        assert!(output.stderr.is_empty());
    }
    assert!(
        started.elapsed() <= CONCURRENT_HOOK_TEST_BUDGET,
        "concurrent plugin hooks exceeded their bounded execution budget"
    );

    let spool = SqliteSpoolStore::for_application(application_paths(&home));
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

#[test]
fn versioned_plugin_matrix_recovers_bounded_spool_and_deduplicates() {
    let Some(target) = supported_target() else {
        return;
    };
    let matrix: serde_json::Value = serde_json::from_str(include_str!(
        "../../lili-session/tests/fixtures/codex/matrix.json"
    ))
    .unwrap();
    let required = matrix["required"].as_array().unwrap();
    assert_eq!(required.len(), 1);
    assert_eq!(required[0]["codexVersion"], "0.147.0");
    let declared = required[0]["fixtures"].as_object().unwrap();
    assert_eq!(
        declared
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        [
            "PermissionRequest",
            "SessionEnd",
            "SessionStart",
            "Stop",
            "UserPromptSubmit",
            "agent-turn-complete",
        ]
        .into_iter()
        .collect()
    );

    let temp = TempDir::new();
    let home = temp.0.join("home");
    let codex_home = temp.0.join("offline codex home");
    let plugin_root = codex_home
        .join("plugins/cache/lili-local/lili")
        .join(env!("CARGO_PKG_VERSION"));
    let launcher = install_plugin_runtime(&plugin_root, target);
    for (surface, fixture) in VERSIONED_FIXTURES {
        let output = invoke(&launcher, &plugin_root, &codex_home, &home, fixture);
        assert_eq!(output.status.code(), Some(0), "{surface} failed");
        assert!(
            output.stdout.is_empty(),
            "{surface} emitted model-visible output"
        );
        assert!(
            output.stderr.is_empty(),
            "{surface} emitted diagnostic output"
        );
        if surface == "Stop" {
            let duplicate = invoke(&launcher, &plugin_root, &codex_home, &home, fixture);
            assert_eq!(duplicate.status.code(), Some(0));
            assert!(duplicate.stdout.is_empty());
            assert!(duplicate.stderr.is_empty());
        }
    }

    let spool = SqliteSpoolStore::for_application(application_paths(&home));

    let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
    let mut recovered = 0;
    let mut duplicates = 0;
    let mut identity_counts = std::collections::BTreeMap::new();
    while let Some(claim) = spool.claim_next(unix_time_ms()).unwrap() {
        let event = claim.event();
        assert_eq!(event.provider.as_str(), "codex");
        assert!(
            event
                .source_discriminator
                .starts_with("plugin:lili@lili-local:0.1.0:hook:")
        );
        assert!(serde_json::to_vec(event).unwrap().len() <= 64 * 1024);
        *identity_counts
            .entry(event.event_id.as_str().to_owned())
            .or_insert(0_u64) += 1;
        if reducer.reduce(event.clone()) == ReductionOutcome::Duplicate {
            duplicates += 1;
        }
        recovered += 1;
        claim.commit().unwrap();
    }
    assert_eq!(recovered, VERSIONED_FIXTURES.len());
    assert_eq!(
        identity_counts
            .values()
            .filter(|count| **count > 1)
            .copied()
            .collect::<Vec<_>>(),
        Vec::<u64>::new()
    );
    assert_eq!(duplicates, 0);
    assert!(spool.claim_next(unix_time_ms()).unwrap().is_none());
    let metrics = spool.metrics().unwrap();
    assert_eq!(metrics.expired_drops, 0);
    assert_eq!(metrics.limit_drops, 0);
    assert_eq!(metrics.malformed_drops, 0);
}

fn install_plugin_runtime(plugin_root: &Path, target: &str) -> PathBuf {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/lili");
    let manifest = plugin_root.join(".codex-plugin/plugin.json");
    fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    fs::write(
        manifest,
        format!(
            r#"{{"name":"lili","version":"{}"}}"#,
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();
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
    home: &Path,
    fixture: &str,
) -> std::process::Output {
    let mut child = Command::new(launcher)
        .env("PLUGIN_ROOT", plugin_root)
        .env(
            "PLUGIN_DATA",
            codex_home
                .join("plugins")
                .join("data")
                .join("lili-lili-local"),
        )
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
        .write_all(fixture.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
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
