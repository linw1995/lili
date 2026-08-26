#[cfg(feature = "acceptance")]
pub mod acceptance_marketplace;
mod desktop_acceptance;
mod desktop_smoke;
mod diagnostics;
pub mod hook_forwarder;
mod integration_cli;
mod ipc_signer;
mod loopback;
#[cfg(target_os = "macos")]
mod macos_panel;
mod platform_pinning;

use std::time::Duration;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use desktop_acceptance::{DesktopAcceptanceState, complete_desktop_acceptance};
use desktop_smoke::{DesktopSmokeState, complete_desktop_smoke};
use ipc_signer::{FETCH_SIGNER_SCRIPT, sign_loopback_request};
use lili_actions::{ActionLoadContext, DEFAULT_GLOBAL_CONCURRENCY, load_actions_file};
use lili_app_state::{
    AppState, AppStateStore, DEFAULT_INGESTION_QUEUE_CAPACITY, DEFAULT_VISIBLE_WINDOW_MARGIN,
    DisplayWorkArea, NativeIngestionActor, NativeIngestionHandle, PersistenceError,
    WindowPlacement, resolve_window_placement,
};
use lili_core::PetId;
use lili_pet::PetCatalog;
use lili_server::{StaticAssets, build_native_router_with_diagnostics_and_persistence};
use lili_session::{
    BoundForwardingEndpoint, ClaimedSqliteSpoolRecord, CodexPluginEvidenceStore,
    ForwardingCredentialStore, ForwardingTransportError, SqliteSpoolStore,
};
#[cfg(test)]
use lili_session::{ClaimedSpoolRecord, SpoolStore};
use loopback::LoopbackServer;
use tauri::{
    Manager, WebviewUrl, WebviewWindowBuilder,
    ipc::CapabilityBuilder,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
};
use tokio::sync::oneshot;

const SPOOL_DRAIN_INTERVAL: Duration = Duration::from_millis(250);
const CONTEXT_MENU_WINDOW_LABEL: &str = "pet-context-menu";
const CONTEXT_MENU_WIDTH: u32 = 196;
const CONTEXT_MENU_HEIGHT: u32 = 112;
const CONTEXT_MENU_INITIALIZATION_SCRIPT: &str = r#"
(() => {
  const invoke = (action) => {
    const nativeInvoke = window.__TAURI_INTERNALS__?.invoke;
    if (!nativeInvoke) return;
    void nativeInvoke('run_pet_context_action', { action });
  };

  for (const button of document.querySelectorAll('[data-action]')) {
    button.onclick = () => invoke(button.dataset.action);
  }
})();
"#;

#[derive(Clone)]
struct PetContextActions {
    state: AppState,
    visibility: CheckMenuItem<tauri::Wry>,
    always_on_top: CheckMenuItem<tauri::Wry>,
}

#[derive(Clone, Default)]
struct PetContextMenuTarget(Arc<AtomicBool>);

#[derive(Clone)]
struct ContextMenuNavigation {
    url: tauri::Url,
    certificate_sha256: [u8; 32],
}

pub use lili_storage::{ApplicationPaths, PathError as ApplicationPathError};

pub fn run() {
    diagnostics::init();
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if let Some(exit_code) = integration_cli::try_run(&arguments) {
        if exit_code != 0 {
            std::process::exit(i32::from(exit_code));
        }
        return;
    }
    let acceptance = std::env::args().any(|argument| argument == "--desktop-acceptance");
    let smoke = acceptance || std::env::args().any(|argument| argument == "--desktop-smoke");
    run_desktop(smoke, acceptance);
}

fn run_desktop(smoke: bool, acceptance: bool) {
    let app = tauri::Builder::default()
        .manage(DesktopAcceptanceState::default())
        .manage(DesktopSmokeState::default())
        .invoke_handler(tauri::generate_handler![
            sign_loopback_request,
            begin_window_drag,
            move_window_to,
            commit_window_position,
            open_pet_context_menu,
            set_pet_context_menu_target,
            run_pet_context_action,
            complete_desktop_acceptance,
            complete_desktop_smoke
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Lili");

    configure_desktop_companion_application(&app)
        .expect("failed to configure desktop companion application");

    let assets = desktop_assets(
        app.path().resource_dir().ok().as_deref(),
        cfg!(debug_assertions),
    )
    .map(StaticAssets::new);
    let application_paths =
        ApplicationPaths::resolve().expect("failed to resolve Lili application storage paths");
    diagnostics::info("storage", "initialize", "legacy_paths_ignored");
    let (state, state_store, saved_window_placement) = match load_app_state(&application_paths) {
        Ok(value) => value,
        Err(_) => {
            diagnostics::error("storage", "open", "unavailable");
            std::process::exit(1);
        }
    };
    app.state::<DesktopAcceptanceState>()
        .configure(application_paths.clone(), state.clone());
    let _native_ingestion = configure_native_runtime(
        !smoke || acceptance,
        state_store.clone(),
        &application_paths,
        &state,
    );
    app.manage(DesktopPersistence {
        store: state_store.clone(),
    });
    app.manage(WindowDragState::default());
    let pets_root = application_paths.pets_root();
    app.manage(PetContextMenuTarget::default());
    let tray_menu =
        setup_tray(&app, state.clone(), &pets_root).expect("failed to configure tray lifecycle");
    app.manage(PetContextActions {
        state: state.clone(),
        visibility: tray_menu.visibility.clone(),
        always_on_top: tray_menu.always_on_top.clone(),
    });
    let loopback = LoopbackServer::bind(build_native_router_with_diagnostics_and_persistence(
        state.clone(),
        assets,
        None,
        state_store.clone(),
    ))
    .expect("failed to bind secure loopback transport");
    let bootstrap_url = loopback.bootstrap_url();
    let certificate_sha256 = loopback.certificate_sha256();
    let origin = loopback.origin();
    let signer = loopback.signer();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    loopback.spawn(shutdown_rx);

    app.manage(signer);
    register_loopback_capability(&app, &origin, smoke, acceptance)
        .expect("failed to register loopback capability");
    register_context_menu_capability(&app, &origin)
        .expect("failed to configure context menu capability");
    app.manage(ContextMenuNavigation {
        url: tauri::Url::parse(&format!(
            "{}/context-menu",
            origin.as_str().trim_end_matches('/')
        ))
        .expect("valid context menu URL"),
        certificate_sha256,
    });
    let window = create_pet_window(&app, &state, &origin, smoke, acceptance)
        .expect("failed to create pet window");
    restore_window_placement(&window, saved_window_placement.as_ref());
    platform_pinning::install_and_navigate(&window, bootstrap_url, certificate_sha256)
        .expect("failed to install loopback certificate pinning");
    window.show().expect("failed to show pet window");
    register_pet_window_events(&window, app.handle().clone());
    run_desktop_event_loop(app, smoke, state, state_store, shutdown_tx);
}

#[cfg(target_os = "macos")]
fn configure_desktop_companion_application(app: &tauri::App) -> tauri::Result<()> {
    let handle = app.handle();
    handle.set_activation_policy(tauri::ActivationPolicy::Accessory)?;
    handle.set_dock_visibility(false)?;
    macos_panel::hide_dock_icon();
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn configure_desktop_companion_application(_app: &tauri::App) -> tauri::Result<()> {
    Ok(())
}

fn configure_native_runtime(
    enabled: bool,
    state_store: Option<AppStateStore>,
    application_paths: &ApplicationPaths,
    state: &AppState,
) -> Option<NativeIngestionHandle> {
    if !enabled {
        return None;
    }
    configure_native_actions(application_paths, state);
    match start_native_ingestion(application_paths, state.clone(), state_store) {
        Ok(handle) => Some(handle),
        Err(_) => {
            diagnostics::warn("ingestion", "start", "transport_unavailable");
            None
        }
    }
}

fn register_loopback_capability(
    app: &tauri::App,
    origin: &tauri::Url,
    smoke: bool,
    acceptance: bool,
) -> tauri::Result<()> {
    let capability = CapabilityBuilder::new("loopback-request-signer")
        .remote(format!("{}/*", origin.as_str().trim_end_matches('/')))
        .local(false)
        .window("pet")
        .permission("allow-sign-loopback-request")
        .permission("allow-begin-window-drag")
        .permission("allow-move-window-to")
        .permission("allow-commit-window-position")
        .permission("allow-open-pet-context-menu")
        .permission("allow-set-pet-context-menu-target");
    let capability = if acceptance {
        capability.permission("allow-complete-desktop-acceptance")
    } else if smoke {
        capability.permission("allow-complete-desktop-smoke")
    } else {
        capability
    };
    app.add_capability(capability)
}

fn register_context_menu_capability(app: &tauri::App, origin: &tauri::Url) -> tauri::Result<()> {
    let capability = CapabilityBuilder::new("context-menu-window")
        .remote(format!("{}/*", origin.as_str().trim_end_matches('/')))
        .local(false)
        .window(CONTEXT_MENU_WINDOW_LABEL)
        .permission("allow-run-pet-context-action");
    app.add_capability(capability)
}

fn create_pet_window(
    app: &tauri::App,
    state: &AppState,
    origin: &tauri::Url,
    smoke: bool,
    acceptance: bool,
) -> tauri::Result<tauri::WebviewWindow> {
    let allowed_origin = origin.origin();
    let always_on_top = tauri::async_runtime::block_on(state.settings()).always_on_top;
    let builder = WebviewWindowBuilder::new(
        app.handle(),
        "pet",
        WebviewUrl::External("about:blank".parse().expect("valid bootstrap URL")),
    )
    .initialization_script(FETCH_SIGNER_SCRIPT)
    .accept_first_mouse(true)
    .title("Lili")
    .inner_size(320.0, 360.0)
    .transparent(true)
    .decorations(false)
    .always_on_top(always_on_top)
    .resizable(false)
    .shadow(false)
    .visible(false)
    .on_navigation(move |url| url.origin() == allowed_origin);
    let window = if acceptance {
        builder
            .initialization_script(desktop_acceptance::SCRIPT)
            .build()
    } else if smoke {
        builder.initialization_script(desktop_smoke::SCRIPT).build()
    } else {
        builder.build()
    }?;
    configure_desktop_companion_window(&window, app.handle().clone())?;
    Ok(window)
}

fn create_context_menu_window(
    app: &tauri::AppHandle,
    navigation: &ContextMenuNavigation,
) -> Result<tauri::WebviewWindow, String> {
    let app_handle = app.clone();
    let has_been_focused = Arc::new(AtomicBool::new(false));
    let focus_state = Arc::clone(&has_been_focused);
    let window = WebviewWindowBuilder::new(
        app,
        CONTEXT_MENU_WINDOW_LABEL,
        WebviewUrl::External("about:blank".parse().expect("valid context menu URL")),
    )
    .initialization_script(CONTEXT_MENU_INITIALIZATION_SCRIPT)
    .title("Lili")
    .inner_size(
        f64::from(CONTEXT_MENU_WIDTH),
        f64::from(CONTEXT_MENU_HEIGHT),
    )
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .resizable(false)
    .shadow(false)
    .skip_taskbar(true)
    .visible(false)
    .focused(false)
    .on_page_load(|window, payload| {
        if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
            let _ = window.eval(CONTEXT_MENU_INITIALIZATION_SCRIPT);
        }
    })
    .build()
    .map_err(|error| format!("failed to create pet context menu window: {error}"))?;
    platform_pinning::install_and_navigate(
        &window,
        navigation.url.clone(),
        navigation.certificate_sha256,
    )
    .map_err(|error| format!("failed to navigate pet context menu window: {error}"))?;
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Focused(true) => {
            focus_state.store(true, Ordering::Release);
        }
        tauri::WindowEvent::Focused(false) if focus_state.swap(false, Ordering::AcqRel) => {
            if let Some(window) = app_handle.get_webview_window(CONTEXT_MENU_WINDOW_LABEL) {
                let _ = window.hide();
            }
        }
        _ => {}
    });
    Ok(window)
}

#[cfg(target_os = "macos")]
fn configure_desktop_companion_window(
    window: &tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> tauri::Result<()> {
    let context_menu_target = app.state::<PetContextMenuTarget>().0.clone();
    macos_panel::configure(window, context_menu_target, move || {
        open_pet_context_menu_from_native(&app);
    })
}

#[cfg(not(target_os = "macos"))]
fn configure_desktop_companion_window(
    _window: &tauri::WebviewWindow,
    _app: tauri::AppHandle,
) -> tauri::Result<()> {
    Ok(())
}

fn restore_window_placement(window: &tauri::WebviewWindow, saved: Option<&WindowPlacement>) {
    if let Some(saved) = saved
        && apply_reachable_window_placement(window, saved).is_err()
    {
        diagnostics::warn("window", "restore_placement", "placement_rejected");
    }
}

fn register_pet_window_events(window: &tauri::WebviewWindow, app: tauri::AppHandle) {
    window.on_window_event(move |event| handle_pet_window_event(&app, event));
}

fn handle_pet_window_event(app: &tauri::AppHandle, event: &tauri::WindowEvent) {
    match event {
        tauri::WindowEvent::CloseRequested { api, .. } => handle_pet_close(app, api),
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::ScaleFactorChanged { .. } => {
            handle_pet_move(app);
        }
        _ => {}
    }
}

fn handle_pet_close(app: &tauri::AppHandle, api: &tauri::CloseRequestApi) {
    api.prevent_close();
    let actions = app.state::<PetContextActions>();
    hide_pet_window(app, &actions.visibility);
}

fn handle_pet_move(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("pet") {
        ensure_window_reachable(&window);
    }
}

fn run_desktop_event_loop(
    app: tauri::App,
    smoke: bool,
    state: AppState,
    state_store: Option<AppStateStore>,
    shutdown_tx: oneshot::Sender<()>,
) {
    let mut shutdown_tx = Some(shutdown_tx);
    app.run(move |app, event| {
        #[cfg(target_os = "macos")]
        if matches!(event, tauri::RunEvent::Ready) {
            macos_panel::hide_dock_icon();
        }
        if matches!(event, tauri::RunEvent::Exit) {
            if !smoke {
                persist_desktop_state(app, &state, state_store.as_ref());
            }
            if let Some(shutdown_tx) = shutdown_tx.take() {
                let _ = shutdown_tx.send(());
            }
        }
    });
}

fn persist_desktop_state(app: &tauri::AppHandle, state: &AppState, store: Option<&AppStateStore>) {
    let Some(store) = store else {
        return;
    };
    let placement = app
        .get_webview_window("pet")
        .as_ref()
        .and_then(current_window_placement);
    let persistent = tauri::async_runtime::block_on(state.persistent_state(placement.clone()));
    if store.save(&persistent).is_err() {
        diagnostics::warn("state", "persist", "write_failed");
    }
    if let Some(placement) = placement
        && store.save_window_placement(&placement).is_err()
    {
        diagnostics::warn("state", "persist_window", "write_failed");
    }
}

fn configure_native_actions(application_paths: &ApplicationPaths, state: &AppState) {
    let context = ActionLoadContext::for_application(application_paths.root());
    let loaded = load_actions_file(&application_paths.actions_path(), &context);
    let enabled_count = loaded.enabled().len();
    let diagnostic_count = loaded.effective().diagnostics.len();
    let configured =
        tauri::async_runtime::block_on(state.configure_actions(loaded, DEFAULT_GLOBAL_CONCURRENCY));
    if configured {
        diagnostics::info_with_counts(
            "actions",
            "configure",
            "configured",
            enabled_count,
            diagnostic_count,
        );
    } else {
        diagnostics::warn("actions", "configure", "supervisor_rejected");
    }
}

type LoadedAppState = (AppState, Option<AppStateStore>, Option<WindowPlacement>);

fn load_app_state(
    application_paths: &ApplicationPaths,
) -> Result<LoadedAppState, PersistenceError> {
    let store = AppStateStore::for_application(application_paths.clone());
    match store.load()? {
        Some(state) => {
            let window_placement = state.window_placement().cloned();
            let pet_catalog = PetCatalog::load_with_selection(
                &application_paths.pets_root(),
                state.selected_pet_id(),
            );
            let state = AppState::with_persistent_state(pet_catalog, state)
                .expect("validated application state must restore");
            Ok((state, Some(store), window_placement))
        }
        None => Ok((
            AppState::with_pet_catalog(PetCatalog::load(&application_paths.pets_root())),
            Some(store),
            None,
        )),
    }
}

fn start_native_ingestion(
    application_paths: &ApplicationPaths,
    state: AppState,
    state_store: Option<AppStateStore>,
) -> Result<NativeIngestionHandle, ForwardingTransportError> {
    let runtime_dir = application_paths.runtime_root();
    let evidence_store = CodexPluginEvidenceStore::for_application(application_paths.clone());
    let previous_credentials = ForwardingCredentialStore::for_runtime_dir(&runtime_dir)
        .load()
        .ok()
        .and_then(|record| record.credentials().ok());
    let codex_adapter = previous_credentials
        .as_ref()
        .and_then(|credentials| evidence_store.load(credentials).ok())
        .unwrap_or_default();
    let (endpoint, handle, actor) = tauri::async_runtime::block_on(async {
        let endpoint = BoundForwardingEndpoint::bind_with_credentials_rotation(
            &runtime_dir,
            |credentials| evidence_store.save(&codex_adapter, credentials),
            || match &previous_credentials {
                Some(credentials) => evidence_store.save(&codex_adapter, credentials),
                None => Ok(()),
            },
        )?;
        let credentials = endpoint.credentials();
        let (handle, actor) = NativeIngestionActor::channel_with_diagnostics_and_evidence_store(
            state,
            credentials,
            DEFAULT_INGESTION_QUEUE_CAPACITY,
            codex_adapter,
            Some(evidence_store),
        )
        .await;
        let actor = match state_store {
            Some(store) => actor.with_persistence_store(store),
            None => actor,
        };
        Ok::<_, ForwardingTransportError>((endpoint, handle, actor))
    })?;
    tauri::async_runtime::spawn(actor.run());
    let spool = SqliteSpoolStore::for_application(application_paths.clone());
    tauri::async_runtime::spawn(run_sqlite_native_services(endpoint, handle.clone(), spool));
    Ok(handle)
}

async fn run_sqlite_native_services(
    endpoint: BoundForwardingEndpoint,
    handle: NativeIngestionHandle,
    spool: SqliteSpoolStore,
) {
    if tokio::task::spawn_blocking({
        let spool = spool.clone();
        move || spool.recover_claims(unix_time_ms())
    })
    .await
    .ok()
    .and_then(Result::ok)
    .is_none()
    {
        diagnostics::warn("spool", "recover_claims", "recovery_failed");
    }
    drain_sqlite_spool(&spool, &handle).await;
    tokio::join!(
        serve_native_ingestion(endpoint, handle.clone()),
        drain_sqlite_spool_continuously(spool, handle),
    );
}

async fn drain_sqlite_spool_continuously(spool: SqliteSpoolStore, handle: NativeIngestionHandle) {
    loop {
        tokio::time::sleep(SPOOL_DRAIN_INTERVAL).await;
        drain_sqlite_spool(&spool, &handle).await;
    }
}

async fn drain_sqlite_spool(spool: &SqliteSpoolStore, handle: &NativeIngestionHandle) {
    while let Some(claim) = next_sqlite_spool_claim(spool).await {
        if !ingest_sqlite_spool_claim(claim, handle).await {
            break;
        }
    }
    publish_sqlite_spool_metrics(spool, handle).await;
}

async fn publish_sqlite_spool_metrics(spool: &SqliteSpoolStore, handle: &NativeIngestionHandle) {
    let spool = spool.clone();
    if let Ok(Ok(metrics)) = tokio::task::spawn_blocking(move || spool.metrics()).await {
        let _ = handle.set_spool_metrics(metrics).await;
    }
}

async fn next_sqlite_spool_claim(spool: &SqliteSpoolStore) -> Option<ClaimedSqliteSpoolRecord> {
    let spool = spool.clone();
    match tokio::task::spawn_blocking(move || spool.claim_next(unix_time_ms())).await {
        Ok(Ok(claim)) => claim,
        Ok(Err(_)) | Err(_) => {
            diagnostics::warn("spool", "claim", "read_failed");
            None
        }
    }
}

async fn ingest_sqlite_spool_claim(
    claim: ClaimedSqliteSpoolRecord,
    handle: &NativeIngestionHandle,
) -> bool {
    match handle.ingest_spooled(claim.event().clone()).await {
        Ok(_) => tokio::task::spawn_blocking(move || claim.commit())
            .await
            .is_ok_and(|result| result.is_ok()),
        Err(_) => {
            let _ = tokio::task::spawn_blocking(move || claim.release()).await;
            false
        }
    }
}

#[cfg(test)]
async fn drain_offline_spool_continuously(spool: SpoolStore, handle: NativeIngestionHandle) {
    loop {
        tokio::time::sleep(SPOOL_DRAIN_INTERVAL).await;
        drain_offline_spool(&spool, &handle).await;
    }
}

#[cfg(test)]
async fn drain_offline_spool(spool: &SpoolStore, handle: &NativeIngestionHandle) {
    while let Some(claim) = next_spool_claim(spool).await {
        if !ingest_spool_claim(claim, handle).await {
            break;
        }
    }
    publish_spool_metrics(spool, handle).await;
}

#[cfg(test)]
async fn publish_spool_metrics(spool: &SpoolStore, handle: &NativeIngestionHandle) {
    let spool = spool.clone();
    if let Ok(Ok(metrics)) = tokio::task::spawn_blocking(move || spool.metrics()).await {
        let _ = handle.set_spool_metrics(metrics).await;
    }
}

#[cfg(test)]
async fn next_spool_claim(spool: &SpoolStore) -> Option<ClaimedSpoolRecord> {
    let spool = spool.clone();
    match tokio::task::spawn_blocking(move || spool.claim_next(unix_time_ms())).await {
        Ok(Ok(claim)) => claim,
        Ok(Err(_)) | Err(_) => {
            diagnostics::warn("spool", "claim", "read_failed");
            None
        }
    }
}

#[cfg(test)]
async fn ingest_spool_claim(claim: ClaimedSpoolRecord, handle: &NativeIngestionHandle) -> bool {
    match handle.ingest_spooled(claim.event().clone()).await {
        Ok(_) => {
            if tokio::task::spawn_blocking(move || claim.commit())
                .await
                .is_ok_and(|result| result.is_ok())
            {
                true
            } else {
                diagnostics::warn("spool", "commit", "commit_failed");
                false
            }
        }
        Err(_) => {
            if !tokio::task::spawn_blocking(move || claim.release())
                .await
                .is_ok_and(|result| result.is_ok())
            {
                diagnostics::warn("spool", "release", "release_failed");
            }
            diagnostics::warn("ingestion", "reduce_spooled", "unavailable");
            false
        }
    }
}

async fn serve_native_ingestion(endpoint: BoundForwardingEndpoint, handle: NativeIngestionHandle) {
    loop {
        match endpoint.accept().await {
            Ok(connection) => {
                let handle = handle.clone();
                tauri::async_runtime::spawn(handle_native_connection(connection, handle));
            }
            Err(_) => {
                let _ = handle.record_transport_rejection().await;
                diagnostics::warn("ingestion", "accept_connection", "transport_rejected");
            }
        }
    }
}

async fn handle_native_connection(
    mut connection: lili_session::ForwardingConnection,
    handle: NativeIngestionHandle,
) {
    let Some(payload) = read_forwarding_payload(&mut connection, &handle).await else {
        return;
    };
    let now_ms = unix_time_ms();
    match handle.ingest(payload, now_ms).await {
        Ok(acknowledgement)
            if connection
                .write_acknowledgement(&acknowledgement)
                .await
                .is_err() =>
        {
            diagnostics::warn("ingestion", "write_acknowledgement", "transport_failed");
        }
        Ok(_) => {}
        Err(_) => diagnostics::warn("ingestion", "verify_message", "message_rejected"),
    }
}

async fn read_forwarding_payload(
    connection: &mut lili_session::ForwardingConnection,
    handle: &NativeIngestionHandle,
) -> Option<Vec<u8>> {
    match connection.read_payload().await {
        Ok(payload) => payload,
        Err(_) => {
            let _ = handle.record_transport_rejection().await;
            diagnostics::warn("ingestion", "read_frame", "transport_rejected");
            return None;
        }
    }
    .into()
}

fn unix_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn current_window_placement(window: &tauri::WebviewWindow) -> Option<WindowPlacement> {
    let position = window.outer_position().ok()?;
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten())?;
    let scale = monitor.scale_factor();
    let work_area = monitor.work_area();
    WindowPlacement::new(
        monitor_id(&monitor),
        (f64::from(position.x - work_area.position.x) / scale).round() as i32,
        (f64::from(position.y - work_area.position.y) / scale).round() as i32,
        (scale * 1_000.0).round() as u32,
    )
    .ok()
}

fn monitor_id(monitor: &tauri::Monitor) -> String {
    monitor.name().cloned().unwrap_or_else(|| {
        let position = monitor.position();
        let size = monitor.size();
        format!(
            "display-{}-{}-{}x{}",
            position.x, position.y, size.width, size.height
        )
    })
}

fn display_work_areas(window: &tauri::WebviewWindow) -> Result<Vec<DisplayWorkArea>, String> {
    let primary_id = window
        .primary_monitor()
        .map_err(|error| error.to_string())?
        .as_ref()
        .map(monitor_id);
    window
        .available_monitors()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|monitor| {
            let id = monitor_id(&monitor);
            let work_area = monitor.work_area();
            DisplayWorkArea::new(
                id.clone(),
                work_area.position.x,
                work_area.position.y,
                work_area.size.width,
                work_area.size.height,
                (monitor.scale_factor() * 1_000.0).round() as u32,
                primary_id.as_deref() == Some(id.as_str()),
            )
            .map_err(|error| error.to_string())
        })
        .collect()
}

fn apply_reachable_window_placement(
    window: &tauri::WebviewWindow,
    saved: &WindowPlacement,
) -> Result<WindowPlacement, String> {
    let resolved = reachable_window_placement(window, saved)?;
    apply_physical_window_position(window, &resolved)?;
    Ok(resolved.placement().clone())
}

fn reachable_window_placement(
    window: &tauri::WebviewWindow,
    saved: &WindowPlacement,
) -> Result<lili_app_state::ResolvedWindowPlacement, String> {
    let size = window.outer_size().map_err(|error| error.to_string())?;
    let displays = display_work_areas(window)?;
    resolve_window_placement(
        saved,
        &displays,
        size.width,
        size.height,
        DEFAULT_VISIBLE_WINDOW_MARGIN,
    )
    .ok_or_else(|| "no display work area is available".to_owned())
}

fn apply_physical_window_position(
    window: &tauri::WebviewWindow,
    resolved: &lili_app_state::ResolvedWindowPlacement,
) -> Result<(), String> {
    let current = window.outer_position().map_err(|error| error.to_string())?;
    if current.x != resolved.physical_x() || current.y != resolved.physical_y() {
        window
            .set_position(tauri::PhysicalPosition::new(
                resolved.physical_x(),
                resolved.physical_y(),
            ))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn ensure_window_reachable(window: &tauri::WebviewWindow) {
    let Some(current) = current_window_placement(window) else {
        return;
    };
    if apply_reachable_window_placement(window, &current).is_err() {
        diagnostics::warn("window", "enforce_reachability", "placement_rejected");
    }
}

#[derive(Clone)]
struct DesktopPersistence {
    store: Option<AppStateStore>,
}

#[derive(Clone, Copy, Debug)]
struct WindowDragAnchor {
    pointer_x: i32,
    pointer_y: i32,
    origin: tauri::PhysicalPosition<i32>,
    scale: f64,
}

#[derive(Default)]
struct WindowDragState(std::sync::Mutex<Option<WindowDragAnchor>>);

#[tauri::command]
fn open_pet_context_menu(
    app: tauri::AppHandle,
    source: tauri::WebviewWindow,
    navigation: tauri::State<'_, ContextMenuNavigation>,
    screen_x: i32,
    screen_y: i32,
) -> Result<(), String> {
    open_pet_context_menu_at(
        &app,
        &source,
        &navigation,
        Some(tauri::PhysicalPosition::new(screen_x, screen_y)),
    )
}

#[tauri::command]
fn set_pet_context_menu_target(
    target: bool,
    state: tauri::State<'_, PetContextMenuTarget>,
) -> Result<(), String> {
    state.0.store(target, Ordering::Release);
    Ok(())
}

#[cfg(target_os = "macos")]
fn open_pet_context_menu_from_native(app: &tauri::AppHandle) {
    let Some(source) = app.get_webview_window("pet") else {
        diagnostics::warn("context_menu", "open", "pet_window_unavailable");
        return;
    };
    let navigation = app.state::<ContextMenuNavigation>();
    if open_pet_context_menu_at(app, &source, &navigation, None).is_err() {
        diagnostics::warn("context_menu", "open", "failed");
    }
}

fn open_pet_context_menu_at(
    app: &tauri::AppHandle,
    source: &tauri::WebviewWindow,
    navigation: &ContextMenuNavigation,
    fallback: Option<tauri::PhysicalPosition<i32>>,
) -> Result<(), String> {
    let _ = source.eval("document.activeElement?.blur()");
    let window = context_menu_window(app, navigation)?;
    let pointer = context_menu_pointer(source, fallback)?;
    let position = context_menu_position(source, &window, pointer);
    position_context_menu(&window, position)
}

fn context_menu_window(
    app: &tauri::AppHandle,
    navigation: &ContextMenuNavigation,
) -> Result<tauri::WebviewWindow, String> {
    app.get_webview_window(CONTEXT_MENU_WINDOW_LABEL)
        .map_or_else(|| create_context_menu_window(app, navigation), Ok)
}

fn context_menu_pointer(
    source: &tauri::WebviewWindow,
    fallback: Option<tauri::PhysicalPosition<i32>>,
) -> Result<tauri::PhysicalPosition<i32>, String> {
    source
        .cursor_position()
        .ok()
        .filter(|point| point.x.is_finite() && point.y.is_finite())
        .map(|point| tauri::PhysicalPosition::new(point.x.round() as i32, point.y.round() as i32))
        .or(fallback)
        .ok_or_else(|| "current cursor position is unavailable".to_owned())
}

fn position_context_menu(
    window: &tauri::WebviewWindow,
    position: tauri::PhysicalPosition<i32>,
) -> Result<(), String> {
    window
        .set_position(position)
        .map_err(|error| format!("failed to position pet context menu: {error}"))?;
    window
        .show()
        .map_err(|error| format!("failed to show pet context menu: {error}"))?;
    let _ = window.eval(CONTEXT_MENU_INITIALIZATION_SCRIPT);
    let _ = window.set_focus();
    Ok(())
}

fn context_menu_position(
    source: &tauri::WebviewWindow,
    popup: &tauri::WebviewWindow,
    pointer: tauri::PhysicalPosition<i32>,
) -> tauri::PhysicalPosition<i32> {
    let size = popup
        .outer_size()
        .ok()
        .filter(|size| size.width > 0 && size.height > 0)
        .unwrap_or_else(|| tauri::PhysicalSize::new(CONTEXT_MENU_WIDTH, CONTEXT_MENU_HEIGHT));
    let monitor = source
        .monitor_from_point(f64::from(pointer.x), f64::from(pointer.y))
        .ok()
        .flatten()
        .or_else(|| source.current_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return pointer;
    };
    let work_area = monitor.work_area();
    let min_x = work_area.position.x;
    let min_y = work_area.position.y;
    let max_x = (work_area.position.x + work_area.size.width as i32 - size.width as i32).max(min_x);
    let max_y =
        (work_area.position.y + work_area.size.height as i32 - size.height as i32).max(min_y);
    tauri::PhysicalPosition::new(pointer.x.clamp(min_x, max_x), pointer.y.clamp(min_y, max_y))
}

#[tauri::command]
fn run_pet_context_action(
    app: tauri::AppHandle,
    actions: tauri::State<'_, PetContextActions>,
    action: String,
) -> Result<(), String> {
    hide_pet_context_menu(&app);
    match TrayAction::parse(&action) {
        TrayAction::ToggleVisibility => toggle_pet_window(&app, &actions.visibility),
        TrayAction::AlwaysOnTop => {
            let enabled = !actions.always_on_top.is_checked().unwrap_or(true);
            set_always_on_top(&app, &actions.state, &actions.always_on_top, enabled);
        }
        TrayAction::Quit => handle_application_tray_action(&app, TrayAction::Quit),
        TrayAction::SelectPet(_) | TrayAction::Unknown => {}
    }
    Ok(())
}

fn hide_pet_context_menu(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(CONTEXT_MENU_WINDOW_LABEL) {
        let _ = window.hide();
    }
}

#[tauri::command]
fn begin_window_drag(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, WindowDragState>,
    screen_x: i32,
    screen_y: i32,
) -> Result<bool, String> {
    begin_window_drag_from(&window, &state, screen_x, screen_y)
}

fn begin_window_drag_from(
    window: &tauri::WebviewWindow,
    state: &WindowDragState,
    screen_x: i32,
    screen_y: i32,
) -> Result<bool, String> {
    let anchor = capture_window_drag_anchor(window, screen_x, screen_y)?;
    *state
        .0
        .lock()
        .map_err(|_| "window drag state is unavailable")? = Some(anchor);
    Ok(true)
}

fn capture_window_drag_anchor(
    window: &tauri::WebviewWindow,
    screen_x: i32,
    screen_y: i32,
) -> Result<WindowDragAnchor, String> {
    validate_screen_position(screen_x, screen_y)?;
    let origin = window.outer_position().map_err(|error| error.to_string())?;
    let scale = valid_window_scale(window.scale_factor().ok())?;
    Ok(WindowDragAnchor {
        pointer_x: screen_x,
        pointer_y: screen_y,
        origin,
        scale,
    })
}

#[tauri::command]
fn move_window_to(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, WindowDragState>,
    screen_x: i32,
    screen_y: i32,
) -> Result<bool, String> {
    move_window_to_from(&window, &state, screen_x, screen_y)
}

fn move_window_to_from(
    window: &tauri::WebviewWindow,
    state: &WindowDragState,
    screen_x: i32,
    screen_y: i32,
) -> Result<bool, String> {
    let anchor = *state
        .0
        .lock()
        .map_err(|_| "window drag state is unavailable")?;
    let Some(anchor) = anchor else {
        return Ok(false);
    };
    let target = absolute_window_drag_target(anchor, screen_x, screen_y)?;
    window
        .set_position(target)
        .map_err(|error| error.to_string())?;
    Ok(true)
}

fn valid_window_scale(scale: Option<f64>) -> Result<f64, String> {
    scale
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .ok_or_else(|| "window scale factor could not be determined".to_owned())
}

fn validate_screen_coordinate(value: i32) -> Result<(), String> {
    if value.unsigned_abs() > 1_000_000 {
        return Err("screen coordinate is outside the supported range".to_owned());
    }
    Ok(())
}

fn validate_screen_position(screen_x: i32, screen_y: i32) -> Result<(), String> {
    validate_screen_coordinate(screen_x)?;
    validate_screen_coordinate(screen_y)
}

fn absolute_window_drag_target(
    anchor: WindowDragAnchor,
    screen_x: i32,
    screen_y: i32,
) -> Result<tauri::PhysicalPosition<i32>, String> {
    validate_screen_position(screen_x, screen_y)?;
    let logical_x = screen_x.saturating_sub(anchor.pointer_x);
    let logical_y = screen_y.saturating_sub(anchor.pointer_y);
    Ok(tauri::PhysicalPosition::new(
        anchor
            .origin
            .x
            .saturating_add((f64::from(logical_x) * anchor.scale).round() as i32),
        anchor
            .origin
            .y
            .saturating_add((f64::from(logical_y) * anchor.scale).round() as i32),
    ))
}

#[tauri::command]
async fn commit_window_position(
    window: tauri::WebviewWindow,
    persistence: tauri::State<'_, DesktopPersistence>,
    drag_state: tauri::State<'_, WindowDragState>,
) -> Result<bool, String> {
    *drag_state
        .0
        .lock()
        .map_err(|_| "window drag state is unavailable")? = None;
    let Some(store) = &persistence.store else {
        return Ok(false);
    };
    let placement = current_window_placement(&window)
        .ok_or_else(|| "window placement could not be determined".to_owned())?;
    store
        .save_window_placement(&placement)
        .map_err(|error| format!("window placement could not be saved: {error}"))?;
    Ok(true)
}

fn setup_tray(app: &tauri::App, state: AppState, pets_root: &Path) -> tauri::Result<TrayMenu> {
    let tray_menu = build_tray_menu(app, &state, pets_root)?;
    let icon_menu = tray_menu.clone();
    let event_menu = tray_menu.clone();
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".to_owned()))?;

    TrayIconBuilder::with_id("lili-tray")
        .icon(icon)
        .tooltip("Lili")
        .show_menu_on_left_click(false)
        .menu(&tray_menu.menu)
        .on_tray_icon_event(move |tray, event| {
            handle_tray_icon_event(tray, event, &icon_menu.visibility);
        })
        .on_menu_event({
            let pets_root = pets_root.to_path_buf();
            move |app, event| {
                handle_tray_menu_event(
                    app,
                    TrayAction::parse(event.id().as_ref()),
                    &state,
                    &pets_root,
                    &event_menu.pet_items,
                    &event_menu.visibility,
                    &event_menu.always_on_top,
                );
            }
        })
        .build(app)?;
    Ok(tray_menu)
}

#[derive(Clone)]
struct TrayMenu {
    menu: Menu<tauri::Wry>,
    visibility: CheckMenuItem<tauri::Wry>,
    always_on_top: CheckMenuItem<tauri::Wry>,
    pet_items: Vec<(PetId, CheckMenuItem<tauri::Wry>)>,
}

fn build_tray_menu(
    app: &tauri::App,
    state: &AppState,
    pets_root: &Path,
) -> tauri::Result<TrayMenu> {
    let parts = TrayMenuParts::new(app, state, pets_root)?;
    let menu = Menu::with_items(
        app,
        &[
            &parts.window.visibility,
            &parts.window.always_on_top,
            &parts.pet.menu,
            &parts.utility.separator,
            &parts.utility.quit,
        ],
    )?;
    Ok(TrayMenu {
        menu,
        visibility: parts.window.visibility,
        always_on_top: parts.window.always_on_top,
        pet_items: parts.pet.items,
    })
}

struct TrayMenuParts {
    window: TrayWindowItems,
    pet: TrayPetItems,
    utility: TrayUtilityItems,
}

impl TrayMenuParts {
    fn new(app: &tauri::App, state: &AppState, _pets_root: &Path) -> tauri::Result<Self> {
        Ok(Self {
            window: build_tray_window_items(app, state)?,
            pet: build_tray_pet_items(app, state)?,
            utility: build_tray_utility_items(app)?,
        })
    }
}

struct TrayWindowItems {
    visibility: CheckMenuItem<tauri::Wry>,
    always_on_top: CheckMenuItem<tauri::Wry>,
}

fn build_tray_window_items(app: &tauri::App, state: &AppState) -> tauri::Result<TrayWindowItems> {
    let visibility = CheckMenuItem::with_id(
        app,
        "toggle-visibility",
        "Show / Hide",
        true,
        true,
        None::<&str>,
    )?;
    let always_on_top = CheckMenuItem::with_id(
        app,
        "always-on-top",
        "Always on Top",
        true,
        tauri::async_runtime::block_on(state.settings()).always_on_top,
        None::<&str>,
    )?;
    Ok(TrayWindowItems {
        visibility,
        always_on_top,
    })
}

struct TrayPetItems {
    menu: Submenu<tauri::Wry>,
    items: Vec<(PetId, CheckMenuItem<tauri::Wry>)>,
}

fn build_tray_pet_items(app: &tauri::App, state: &AppState) -> tauri::Result<TrayPetItems> {
    let pet_menu = Submenu::with_id(app, "pets", "Pet", true)?;
    let selected_pet = tauri::async_runtime::block_on(state.snapshot())
        .selected_pet
        .map(|pet| pet.id);
    let pet_items = tauri::async_runtime::block_on(state.available_pets())
        .into_iter()
        .map(|pet| {
            let id = format!("pet:{}", pet.id.as_str());
            let checked = selected_pet.as_ref() == Some(&pet.id);
            CheckMenuItem::with_id(app, id, pet.display_name, true, checked, None::<&str>)
                .map(|item| (pet.id, item))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for (_, item) in &pet_items {
        pet_menu.append(item)?;
    }
    Ok(TrayPetItems {
        menu: pet_menu,
        items: pet_items,
    })
}

struct TrayUtilityItems {
    separator: PredefinedMenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

fn build_tray_utility_items(app: &tauri::App) -> tauri::Result<TrayUtilityItems> {
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    Ok(TrayUtilityItems { separator, quit })
}

fn handle_tray_icon_event(
    tray: &tauri::tray::TrayIcon,
    event: TrayIconEvent,
    visibility: &CheckMenuItem<tauri::Wry>,
) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        ..
    } = event
    {
        toggle_pet_window(tray.app_handle(), visibility);
    }
}

fn handle_tray_menu_event(
    app: &tauri::AppHandle,
    action: TrayAction,
    state: &AppState,
    pets_root: &Path,
    pet_items: &[(PetId, CheckMenuItem<tauri::Wry>)],
    visibility: &CheckMenuItem<tauri::Wry>,
    always_on_top: &CheckMenuItem<tauri::Wry>,
) {
    match action {
        TrayAction::ToggleVisibility | TrayAction::AlwaysOnTop => {
            handle_window_tray_action(app, action, state, visibility, always_on_top);
        }
        TrayAction::SelectPet(pet_id) => {
            handle_pet_selection(app, state, pets_root, pet_items, visibility, &pet_id);
        }
        TrayAction::Quit | TrayAction::Unknown => {
            handle_application_tray_action(app, action);
        }
    }
}

fn handle_window_tray_action(
    app: &tauri::AppHandle,
    action: TrayAction,
    state: &AppState,
    visibility: &CheckMenuItem<tauri::Wry>,
    always_on_top: &CheckMenuItem<tauri::Wry>,
) {
    match action {
        TrayAction::ToggleVisibility => toggle_pet_window(app, visibility),
        TrayAction::AlwaysOnTop => update_always_on_top(app, state, always_on_top),
        _ => {}
    }
}

fn show_pet_window(app: &tauri::AppHandle, visibility: &CheckMenuItem<tauri::Wry>) {
    if let Some(window) = app.get_webview_window("pet") {
        if window.show().is_ok() {
            let _ = visibility.set_checked(true);
            let _ = window.set_focus();
        }
    }
}

fn hide_pet_window(app: &tauri::AppHandle, visibility: &CheckMenuItem<tauri::Wry>) {
    if let Some(window) = app.get_webview_window("pet") {
        if window.hide().is_ok() {
            let _ = visibility.set_checked(false);
        }
    }
}

fn toggle_pet_window(app: &tauri::AppHandle, visibility: &CheckMenuItem<tauri::Wry>) {
    let Some(window) = app.get_webview_window("pet") else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        hide_pet_window(app, visibility);
    } else {
        show_pet_window(app, visibility);
    }
}

fn update_always_on_top(
    app: &tauri::AppHandle,
    state: &AppState,
    always_on_top: &CheckMenuItem<tauri::Wry>,
) {
    let enabled = always_on_top.is_checked().unwrap_or(true);
    set_always_on_top(app, state, always_on_top, enabled);
}

fn set_always_on_top(
    app: &tauri::AppHandle,
    state: &AppState,
    always_on_top: &CheckMenuItem<tauri::Wry>,
    enabled: bool,
) {
    let _ = always_on_top.set_checked(enabled);
    if let Some(window) = app.get_webview_window("pet") {
        let _ = window.set_always_on_top(enabled);
    }
    let state = state.clone();
    tauri::async_runtime::spawn(async move {
        let mut settings = state.settings().await;
        settings.always_on_top = enabled;
        state.replace_settings(settings).await;
    });
}

fn handle_pet_selection(
    app: &tauri::AppHandle,
    state: &AppState,
    pets_root: &Path,
    pet_items: &[(PetId, CheckMenuItem<tauri::Wry>)],
    visibility: &CheckMenuItem<tauri::Wry>,
    pet_id: &PetId,
) {
    let persistence = app.state::<DesktopPersistence>();
    if select_pet(state, pets_root, pet_id, persistence.store.as_ref()).is_err() {
        return;
    }
    for (candidate, item) in pet_items {
        let _ = item.set_checked(candidate == pet_id);
    }
    if let Some(window) = app.get_webview_window("pet") {
        if window.show().is_ok() {
            let _ = visibility.set_checked(true);
        }
    }
}

fn handle_application_tray_action(app: &tauri::AppHandle, action: TrayAction) {
    if action == TrayAction::Quit {
        app.exit(0);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TrayAction {
    ToggleVisibility,
    AlwaysOnTop,
    SelectPet(PetId),
    Quit,
    Unknown,
}

impl TrayAction {
    fn parse(id: &str) -> Self {
        match id {
            "toggle-visibility" => Self::ToggleVisibility,
            "always-on-top" => Self::AlwaysOnTop,
            "quit" => Self::Quit,
            _ => id
                .strip_prefix("pet:")
                .and_then(PetId::parse)
                .map_or(Self::Unknown, Self::SelectPet),
        }
    }
}

fn select_pet(
    state: &AppState,
    pets_root: &Path,
    pet_id: &PetId,
    store: Option<&AppStateStore>,
) -> Result<(), String> {
    let catalog = PetCatalog::load_with_selection(pets_root, Some(pet_id));
    if catalog.active().definition().id() != pet_id {
        return Err("selected pet is unavailable".to_owned());
    }
    if let Some(store) = store {
        let persistent = tauri::async_runtime::block_on(state.persistent_state(None))
            .with_selected_pet_id(Some(pet_id.clone()));
        store
            .save_selected_pet(&persistent)
            .map_err(|error| format!("selected pet could not be saved: {error}"))?;
    }
    tauri::async_runtime::block_on(state.replace_pet_catalog(catalog));
    Ok(())
}

fn desktop_assets(
    resource_dir: Option<&Path>,
    allow_development_fallback: bool,
) -> Option<PathBuf> {
    resource_dir
        .map(|root| root.join("web"))
        .filter(|assets| assets.is_dir())
        .or_else(|| {
            allow_development_fallback
                .then(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist"))
                .filter(|assets| assets.is_dir())
        })
}

#[cfg(test)]
mod tests {
    use lili_session::{
        ForwardingCredentials, ProviderCapabilitiesInputV1, ProviderInputV1, SpoolEnqueueOutcome,
        SpoolLimits, normalize_provider_input,
    };

    use super::*;

    #[tokio::test]
    async fn live_spool_drainer_ingests_events_enqueued_after_startup() {
        let root =
            std::env::temp_dir().join(format!("lili-live-spool-drain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let spool = SpoolStore::new(root.join("spool"), SpoolLimits::default());
        let state = AppState::default();
        let credentials = ForwardingCredentials::generate().unwrap();
        let (handle, actor) = NativeIngestionActor::channel(
            state.clone(),
            credentials,
            DEFAULT_INGESTION_QUEUE_CAPACITY,
        )
        .await;
        let actor_task = tokio::spawn(actor.run());

        drain_offline_spool(&spool, &handle).await;
        let mut snapshots = handle.subscribe();
        let drainer_task = tokio::spawn(drain_offline_spool_continuously(
            spool.clone(),
            handle.clone(),
        ));
        let now_ms = unix_time_ms();
        let event = normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("turn_completed".to_owned()),
            event_id: Some("event-after-startup".to_owned()),
            session_id: Some("session-after-startup".to_owned()),
            turn_id: Some("turn-after-startup".to_owned()),
            occurred_at_ms: Some(now_ms),
            project: None,
            summary: None,
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: Some("hook:Stop".to_owned()),
        })
        .unwrap();
        assert_eq!(
            spool.enqueue(&event, now_ms).unwrap(),
            SpoolEnqueueOutcome::Stored
        );

        tokio::time::timeout(Duration::from_secs(2), snapshots.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(snapshots.borrow().revision, 1);
        assert!(spool.claim_next(unix_time_ms()).unwrap().is_none());

        drainer_task.abort();
        drainer_task.await.unwrap_err();
        drop(handle);
        actor_task.await.unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spool_lock_backoff_does_not_block_async_runtime() {
        use std::os::unix::fs::DirBuilderExt;

        let root =
            std::env::temp_dir().join(format!("lili-spool-lock-backoff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let spool = SpoolStore::new(root.join("spool"), SpoolLimits::default());
        spool.metrics().unwrap();
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(spool.directory().join(".lock"))
            .unwrap();

        let claim_spool = spool.clone();
        let claim_task = tokio::spawn(async move { next_spool_claim(&claim_spool).await });
        tokio::time::timeout(
            Duration::from_millis(100),
            tokio::time::sleep(Duration::from_millis(10)),
        )
        .await
        .unwrap();

        std::fs::remove_dir(spool.directory().join(".lock")).unwrap();
        assert!(claim_task.await.unwrap().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn packaged_assets_are_preferred() {
        let root = std::env::temp_dir().join(format!("lili-assets-{}", std::process::id()));
        let assets = root.join("web");
        std::fs::create_dir_all(&assets).unwrap();
        assert_eq!(desktop_assets(Some(&root), false), Some(assets));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn release_mode_fails_closed_without_packaged_assets() {
        assert_eq!(desktop_assets(None, false), None);
    }

    #[test]
    fn tray_actions_reject_untrusted_pet_identifiers() {
        assert_eq!(
            TrayAction::parse("toggle-visibility"),
            TrayAction::ToggleVisibility
        );
        assert_eq!(TrayAction::parse("always-on-top"), TrayAction::AlwaysOnTop);
        assert_eq!(TrayAction::parse("show"), TrayAction::Unknown);
        assert_eq!(TrayAction::parse("hide"), TrayAction::Unknown);
        assert_eq!(TrayAction::parse("settings"), TrayAction::Unknown);
        assert_eq!(TrayAction::parse("diagnostics"), TrayAction::Unknown);
        assert_eq!(
            TrayAction::parse("pet:lili"),
            TrayAction::SelectPet(PetId::parse("lili").unwrap())
        );
        assert_eq!(TrayAction::parse("pet:bad\nvalue"), TrayAction::Unknown);
        assert_eq!(TrayAction::parse("unknown"), TrayAction::Unknown);
    }

    #[test]
    fn desktop_state_loading_does_not_touch_codex_home() {
        let root = std::env::temp_dir().join(format!(
            "lili-desktop-storage-isolation-{}",
            uuid::Uuid::new_v4()
        ));
        let application_paths = ApplicationPaths::from_root(root.join("application")).unwrap();
        let codex_home = root.join("codex-home");
        std::fs::create_dir_all(&codex_home).unwrap();
        let marker = codex_home.join("marker");
        std::fs::write(&marker, b"untouched").unwrap();

        let (_state, store, placement) = load_app_state(&application_paths).unwrap();

        assert!(store.is_some());
        assert!(placement.is_none());
        assert_eq!(std::fs::read(&marker).unwrap(), b"untouched");
        assert!(!codex_home.join("lili").exists());
        assert!(!codex_home.join("pets").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn absolute_window_drag_target_is_anchored_and_idempotent() {
        let anchor = WindowDragAnchor {
            pointer_x: 100,
            pointer_y: 200,
            origin: tauri::PhysicalPosition::new(800, 400),
            scale: 2.0,
        };
        let target = absolute_window_drag_target(anchor, 130, 180).unwrap();
        assert_eq!(target, tauri::PhysicalPosition::new(860, 360));
        assert_eq!(
            absolute_window_drag_target(anchor, 130, 180).unwrap(),
            target
        );
        assert_eq!(
            absolute_window_drag_target(anchor, -400, 900).unwrap(),
            tauri::PhysicalPosition::new(-200, 1_800)
        );
        assert!(absolute_window_drag_target(anchor, 1_000_001, 0).is_err());
        assert!(valid_window_scale(Some(0.0)).is_err());
    }
}
