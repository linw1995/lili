mod desktop_smoke;
pub mod hook_forwarder;
mod integration_cli;
mod ipc_signer;
mod loopback;
mod platform_pinning;

use std::path::{Path, PathBuf};

use desktop_smoke::{DesktopSmokeState, complete_desktop_smoke};
use ipc_signer::{FETCH_SIGNER_SCRIPT, sign_loopback_request};
use lili_app_state::{
    AppState, AppStateStore, DEFAULT_INGESTION_QUEUE_CAPACITY, NativeIngestionActor,
    NativeIngestionHandle, WindowPlacement,
};
use lili_pet::{PetCatalog, resolve_codex_home};
use lili_server::{StaticAssets, build_router};
use lili_session::{BoundForwardingEndpoint, ForwardingTransportError, SpoolStore};
use loopback::LoopbackServer;
use tauri::{
    Manager, WebviewUrl, WebviewWindowBuilder,
    ipc::CapabilityBuilder,
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
};
use tokio::sync::oneshot;

pub fn run() {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if let Some(exit_code) = integration_cli::try_run(&arguments) {
        if exit_code != 0 {
            std::process::exit(i32::from(exit_code));
        }
        return;
    }
    let smoke = std::env::args().any(|argument| argument == "--desktop-smoke");
    run_desktop(smoke);
}

fn run_desktop(smoke: bool) {
    let app = tauri::Builder::default()
        .manage(DesktopSmokeState::default())
        .invoke_handler(tauri::generate_handler![
            sign_loopback_request,
            commit_window_position,
            complete_desktop_smoke
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Lili");

    setup_tray(&app).expect("failed to configure tray lifecycle");
    let assets = desktop_assets(
        app.path().resource_dir().ok().as_deref(),
        cfg!(debug_assertions),
    )
    .map(StaticAssets::new);
    let (state, state_store, codex_home) = load_app_state();
    app.manage(DesktopPersistence {
        state: state.clone(),
        store: state_store.clone(),
    });
    if !smoke
        && let Some(codex_home) = codex_home
        && let Err(error) = start_native_ingestion(&codex_home, state.clone())
    {
        tracing::warn!(%error, "native event ingestion was not started");
    }
    let loopback = LoopbackServer::bind(build_router(state.clone(), assets))
        .expect("failed to bind secure loopback transport");
    let bootstrap_url = loopback.bootstrap_url();
    let certificate_sha256 = loopback.certificate_sha256();
    let origin = loopback.origin();
    let signer = loopback.signer();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    loopback.spawn(shutdown_rx);

    app.manage(signer);
    app.add_capability({
        let capability = CapabilityBuilder::new("loopback-request-signer")
            .remote(format!("{}/*", origin.as_str().trim_end_matches('/')))
            .local(false)
            .window("pet")
            .permission("allow-sign-loopback-request")
            .permission("allow-commit-window-position")
            .permission("core:window:allow-start-dragging");
        if smoke {
            capability.permission("allow-complete-desktop-smoke")
        } else {
            capability
        }
    })
    .expect("failed to register loopback capability");

    let allowed_origin = origin.origin();
    let mut builder = WebviewWindowBuilder::new(
        app.handle(),
        "pet",
        WebviewUrl::External("about:blank".parse().expect("valid bootstrap URL")),
    )
    .initialization_script(FETCH_SIGNER_SCRIPT)
    .title("Lili")
    .inner_size(320.0, 360.0)
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .resizable(false)
    .shadow(false)
    .visible(false)
    .on_navigation(move |url| url.origin() == allowed_origin);
    if smoke {
        builder = builder.initialization_script(desktop_smoke::SCRIPT);
    }
    let window = builder.build().expect("failed to create pet window");
    platform_pinning::install_and_navigate(&window, bootstrap_url, certificate_sha256)
        .expect("failed to install loopback certificate pinning");
    window.show().expect("failed to show pet window");

    let close_app = app.handle().clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if let Some(window) = close_app.get_webview_window("pet") {
                let _ = window.hide();
            }
        }
    });

    let mut shutdown_tx = Some(shutdown_tx);
    app.run(move |app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            if !smoke && let Some(store) = &state_store {
                let placement = app
                    .get_webview_window("pet")
                    .as_ref()
                    .and_then(current_window_placement);
                let persistent = tauri::async_runtime::block_on(state.persistent_state(placement));
                if let Err(error) = store.save(&persistent) {
                    tracing::warn!(%error, "application state was not persisted");
                }
            }
            if let Some(shutdown_tx) = shutdown_tx.take() {
                let _ = shutdown_tx.send(());
            }
        }
    });
}

fn load_app_state() -> (AppState, Option<AppStateStore>, Option<PathBuf>) {
    let codex_home = match resolve_codex_home() {
        Ok(codex_home) => codex_home,
        Err(error) => {
            tracing::warn!(%error, "Codex home was not resolved");
            return (AppState::default(), None, None);
        }
    };
    let store = AppStateStore::for_codex_home(&codex_home);
    match store.load() {
        Ok(Some(state)) => {
            let pet_catalog = PetCatalog::load_with_selection(&codex_home, state.selected_pet_id());
            let state = AppState::with_persistent_state(pet_catalog, state)
                .expect("validated application state must restore");
            (state, Some(store), Some(codex_home))
        }
        Ok(None) => {
            let state = AppState::with_pet_catalog(PetCatalog::load(&codex_home));
            (state, Some(store), Some(codex_home))
        }
        Err(error) => {
            tracing::warn!(%error, "persisted application state was ignored");
            (
                AppState::with_pet_catalog(PetCatalog::load(&codex_home)),
                None,
                Some(codex_home),
            )
        }
    }
}

fn start_native_ingestion(
    codex_home: &Path,
    state: AppState,
) -> Result<(), ForwardingTransportError> {
    let runtime_dir = codex_home.join("lili").join("runtime");
    let (endpoint, handle, actor) = tauri::async_runtime::block_on(async {
        let endpoint = BoundForwardingEndpoint::bind(&runtime_dir)?;
        let credentials = endpoint.credentials();
        let (handle, actor) =
            NativeIngestionActor::channel(state, credentials, DEFAULT_INGESTION_QUEUE_CAPACITY)
                .await;
        Ok::<_, ForwardingTransportError>((endpoint, handle, actor))
    })?;
    tauri::async_runtime::spawn(actor.run());
    let spool = SpoolStore::for_codex_home(codex_home);
    tauri::async_runtime::spawn(run_native_services(endpoint, handle, spool));
    Ok(())
}

async fn run_native_services(
    endpoint: BoundForwardingEndpoint,
    handle: NativeIngestionHandle,
    spool: SpoolStore,
) {
    if let Err(error) = spool.recover_claims() {
        tracing::warn!(%error, "offline event claims were not recovered");
    }
    drain_offline_spool(&spool, &handle).await;
    serve_native_ingestion(endpoint, handle).await;
}

async fn drain_offline_spool(spool: &SpoolStore, handle: &NativeIngestionHandle) {
    if let Ok(metrics) = spool.metrics() {
        let _ = handle.set_spool_metrics(metrics).await;
    }
    loop {
        let claim = match spool.claim_next(unix_time_ms()) {
            Ok(Some(claim)) => claim,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(%error, "offline event spool could not be read");
                break;
            }
        };
        match handle.ingest_spooled(claim.event().clone()).await {
            Ok(_) => {
                if let Err(error) = claim.commit() {
                    tracing::warn!(%error, "accepted offline event was not committed");
                    break;
                }
            }
            Err(error) => {
                if let Err(release_error) = claim.release() {
                    tracing::warn!(%release_error, "offline event claim was not released");
                }
                tracing::warn!(%error, "offline event ingestion became unavailable");
                break;
            }
        }
    }
    if let Ok(metrics) = spool.metrics() {
        let _ = handle.set_spool_metrics(metrics).await;
    }
}

async fn serve_native_ingestion(endpoint: BoundForwardingEndpoint, handle: NativeIngestionHandle) {
    loop {
        match endpoint.accept().await {
            Ok(connection) => {
                let handle = handle.clone();
                tauri::async_runtime::spawn(handle_native_connection(connection, handle));
            }
            Err(error) => {
                let _ = handle.record_transport_rejection().await;
                tracing::warn!(%error, "native event connection was rejected");
            }
        }
    }
}

async fn handle_native_connection(
    mut connection: lili_session::ForwardingConnection,
    handle: NativeIngestionHandle,
) {
    let payload = match connection.read_payload().await {
        Ok(payload) => payload,
        Err(error) => {
            let _ = handle.record_transport_rejection().await;
            tracing::warn!(%error, "native event frame was rejected");
            return;
        }
    };
    let now_ms = unix_time_ms();
    match handle.ingest(payload, now_ms).await {
        Ok(acknowledgement) => {
            if let Err(error) = connection.write_acknowledgement(&acknowledgement).await {
                tracing::warn!(%error, "native event acknowledgement failed");
            }
        }
        Err(error) => tracing::warn!(%error, "native event message was rejected"),
    }
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
    let scale = window.scale_factor().ok()?;
    let display_id = window
        .current_monitor()
        .ok()
        .flatten()
        .and_then(|monitor| monitor.name().cloned())
        .unwrap_or_else(|| "unknown-display".to_owned());
    WindowPlacement::new(
        display_id,
        (f64::from(position.x) / scale).round() as i32,
        (f64::from(position.y) / scale).round() as i32,
        (scale * 1_000.0).round() as u32,
    )
    .ok()
}

#[derive(Clone)]
struct DesktopPersistence {
    state: AppState,
    store: Option<AppStateStore>,
}

#[tauri::command]
async fn commit_window_position(
    window: tauri::WebviewWindow,
    persistence: tauri::State<'_, DesktopPersistence>,
) -> Result<bool, String> {
    let Some(store) = &persistence.store else {
        return Ok(false);
    };
    let placement = current_window_placement(&window)
        .ok_or_else(|| "window placement could not be determined".to_owned())?;
    let state = persistence.state.persistent_state(Some(placement)).await;
    store
        .save(&state)
        .map_err(|error| format!("window placement could not be saved: {error}"))?;
    Ok(true)
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "Hide", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &hide, &quit])?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".to_owned()))?;

    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("Lili")
        .show_menu_on_left_click(false)
        .menu(&menu)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
                && let Some(window) = tray.app_handle().get_webview_window("pet")
            {
                let visible = window.is_visible().unwrap_or(false);
                let _ = if visible {
                    window.hide()
                } else {
                    window.show()
                };
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("pet") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "hide" => {
                if let Some(window) = app.get_webview_window("pet") {
                    let _ = window.hide();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
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
    use super::*;

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
}
