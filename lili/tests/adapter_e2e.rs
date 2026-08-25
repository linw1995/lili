#![cfg(unix)]

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use lili_app_state::{
    AppState, DEFAULT_INGESTION_QUEUE_CAPACITY, NativeIngestionActor, NativeIngestionHandle,
};
use lili_session::{
    BoundForwardingEndpoint, CodexIntegrationSurface, ForwardingConnection, NotificationKind,
};
use lili_storage::ApplicationPaths;

const NOTIFY_FIXTURE: &str =
    include_str!("../../lili-session/tests/fixtures/codex/0.147.0/agent-turn-complete.json");
const PERMISSION_FIXTURE: &str =
    include_str!("../../lili-session/tests/fixtures/codex/0.147.0/permission-request.json");
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(format!(
            "/tmp/lili-adapter-e2e-{}-{sequence}",
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn packaged_notify_argv_reaches_one_native_completion_notification() {
    run_fixture(
        HookInvocation::JsonArgv,
        NOTIFY_FIXTURE,
        NotificationKind::Completion,
        CodexIntegrationSurface::Notify,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn packaged_lifecycle_stdin_reaches_one_native_attention_notification() {
    run_fixture(
        HookInvocation::JsonStdin,
        PERMISSION_FIXTURE,
        NotificationKind::Attention,
        CodexIntegrationSurface::PermissionRequest,
    )
    .await;
}

#[derive(Clone, Copy)]
enum HookInvocation {
    JsonArgv,
    JsonStdin,
}

async fn run_fixture(
    invocation: HookInvocation,
    fixture: &'static str,
    expected_kind: NotificationKind,
    expected_surface: CodexIntegrationSurface,
) {
    let temp = TempDir::new();
    let home = short_home();
    let application_paths = application_paths(&home);
    let state = AppState::default();
    let endpoint = BoundForwardingEndpoint::bind(&application_paths.runtime_root()).unwrap();
    let (handle, actor) = NativeIngestionActor::channel(
        state.clone(),
        endpoint.credentials(),
        DEFAULT_INGESTION_QUEUE_CAPACITY,
    )
    .await;
    let actor_task = tokio::spawn(actor.run());
    let server_task = tokio::spawn(serve_one(endpoint, handle.clone()));

    let output = tokio::task::spawn_blocking({
        let home = home.clone();
        let codex_home = temp.0.join("codex-home");
        move || invoke_packaged_hook(&home, &codex_home, invocation, fixture)
    })
    .await
    .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    server_task.await.unwrap();

    let snapshot = state.snapshot().await;
    assert_eq!(snapshot.session_state.notifications.len(), 1);
    assert_eq!(snapshot.session_state.notifications[0].kind, expected_kind);
    let diagnostics = state.ingestion_diagnostics().await;
    assert_eq!(diagnostics.accepted_messages, 1);
    assert_eq!(
        diagnostics
            .codex_adapter
            .last_accepted_event
            .as_ref()
            .unwrap()
            .surface,
        expected_surface
    );
    assert!(!application_paths.database_path().exists());

    drop(handle);
    actor_task.await.unwrap();
    fs::remove_dir_all(home).unwrap();
}

async fn serve_one(endpoint: BoundForwardingEndpoint, handle: NativeIngestionHandle) {
    let connection = endpoint.accept().await.unwrap();
    ingest_one(connection, handle).await;
}

async fn ingest_one(mut connection: ForwardingConnection, handle: NativeIngestionHandle) {
    let payload = connection.read_payload().await.unwrap();
    let acknowledgement = handle.ingest(payload, unix_time_ms()).await.unwrap();
    connection
        .write_acknowledgement(&acknowledgement)
        .await
        .unwrap();
}

fn invoke_packaged_hook(
    home: &Path,
    codex_home: &Path,
    invocation: HookInvocation,
    fixture: &str,
) -> std::process::Output {
    match invocation {
        HookInvocation::JsonArgv => Command::new(env!("CARGO_BIN_EXE_lili-hook"))
            .args(["--json-argv", fixture])
            .env("CODEX_HOME", codex_home)
            .env("HOME", home)
            .env("XDG_STATE_HOME", home.join("state"))
            .output()
            .unwrap(),
        HookInvocation::JsonStdin => {
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
                .write_all(fixture.as_bytes())
                .unwrap();
            child.wait_with_output().unwrap()
        }
    }
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
        "/tmp/lili-adapter-home-{}-{sequence}",
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
