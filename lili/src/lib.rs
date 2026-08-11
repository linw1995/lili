mod desktop_acceptance;
mod desktop_smoke;
mod diagnostics;
pub mod hook_forwarder;
mod integration_cli;
mod ipc_signer;
mod loopback;
mod platform_pinning;

use std::path::{Path, PathBuf};

use desktop_acceptance::{DesktopAcceptanceState, complete_desktop_acceptance};
use desktop_smoke::{DesktopSmokeState, complete_desktop_smoke};
use ipc_signer::{FETCH_SIGNER_SCRIPT, sign_loopback_request};
use lili_actions::{
    ActionLoadContext, DEFAULT_GLOBAL_CONCURRENCY, action_config_path, load_actions_file,
};
use lili_app_state::{
    AppState, AppStateStore, DEFAULT_INGESTION_QUEUE_CAPACITY, DEFAULT_VISIBLE_WINDOW_MARGIN,
    DisplayWorkArea, NativeIngestionActor, NativeIngestionHandle, WindowPlacement,
    resolve_window_placement,
};
use lili_core::PetId;
use lili_integration::{IntegrationKind, inspect};
use lili_pet::{PetCatalog, persist_selected_pet, resolve_codex_home};
use lili_server::{StaticAssets, build_native_router};
use lili_session::{BoundForwardingEndpoint, ForwardingTransportError, SpoolStore};
use loopback::LoopbackServer;
use tauri::{
    Manager, WebviewUrl, WebviewWindowBuilder,
    ipc::CapabilityBuilder,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
};
use tokio::sync::oneshot;

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
            commit_window_position,
            complete_desktop_acceptance,
            complete_desktop_smoke
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Lili");

    let assets = desktop_assets(
        app.path().resource_dir().ok().as_deref(),
        cfg!(debug_assertions),
    )
    .map(StaticAssets::new);
    let (state, state_store, codex_home, saved_window_placement) = load_app_state();
    app.state::<DesktopAcceptanceState>()
        .configure(codex_home.clone());
    if (!smoke || acceptance)
        && let Some(codex_home) = codex_home.as_deref()
    {
        configure_native_actions(codex_home, &state);
    }
    app.manage(DesktopPersistence {
        state: state.clone(),
        store: state_store.clone(),
    });
    setup_tray(&app, state.clone(), codex_home.as_deref())
        .expect("failed to configure tray lifecycle");
    if (!smoke || acceptance)
        && let Some(codex_home) = codex_home.as_deref()
        && start_native_ingestion(codex_home, state.clone()).is_err()
    {
        diagnostics::warn("ingestion", "start", "transport_unavailable");
    }
    let loopback = LoopbackServer::bind(build_native_router(state.clone(), assets))
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
        if acceptance {
            capability.permission("allow-complete-desktop-acceptance")
        } else if smoke {
            capability.permission("allow-complete-desktop-smoke")
        } else {
            capability
        }
    })
    .expect("failed to register loopback capability");

    let allowed_origin = origin.origin();
    let always_on_top = tauri::async_runtime::block_on(state.settings()).always_on_top;
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
    .always_on_top(always_on_top)
    .resizable(false)
    .shadow(false)
    .visible(false)
    .on_navigation(move |url| url.origin() == allowed_origin);
    if acceptance {
        builder = builder.initialization_script(desktop_acceptance::SCRIPT);
    } else if smoke {
        builder = builder.initialization_script(desktop_smoke::SCRIPT);
    }
    let window = builder.build().expect("failed to create pet window");
    if let Some(saved_window_placement) = saved_window_placement.as_ref()
        && apply_reachable_window_placement(&window, saved_window_placement).is_err()
    {
        diagnostics::warn("window", "restore_placement", "placement_rejected");
    }
    platform_pinning::install_and_navigate(&window, bootstrap_url, certificate_sha256)
        .expect("failed to install loopback certificate pinning");
    window.show().expect("failed to show pet window");

    let close_app = app.handle().clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if let Some(window) = close_app.get_webview_window("pet") {
                let _ = window.hide();
            }
        }
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::ScaleFactorChanged { .. } => {
            if let Some(window) = close_app.get_webview_window("pet") {
                ensure_window_reachable(&window);
            }
        }
        _ => {}
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
                if store.save(&persistent).is_err() {
                    diagnostics::warn("state", "persist", "write_failed");
                }
            }
            if let Some(shutdown_tx) = shutdown_tx.take() {
                let _ = shutdown_tx.send(());
            }
        }
    });
}

fn configure_native_actions(codex_home: &Path, state: &AppState) {
    let context = ActionLoadContext::for_codex_home(codex_home);
    let loaded = load_actions_file(&action_config_path(codex_home), &context);
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

fn load_app_state() -> (
    AppState,
    Option<AppStateStore>,
    Option<PathBuf>,
    Option<WindowPlacement>,
) {
    let codex_home = match resolve_codex_home() {
        Ok(codex_home) => codex_home,
        Err(_) => {
            diagnostics::warn("configuration", "resolve_home", "invalid_home");
            return (AppState::default(), None, None, None);
        }
    };
    let store = AppStateStore::for_codex_home(&codex_home);
    match store.load() {
        Ok(Some(state)) => {
            let window_placement = state.window_placement().cloned();
            let pet_catalog = PetCatalog::load_with_selection(&codex_home, state.selected_pet_id());
            let state = AppState::with_persistent_state(pet_catalog, state)
                .expect("validated application state must restore");
            (state, Some(store), Some(codex_home), window_placement)
        }
        Ok(None) => {
            let state = AppState::with_pet_catalog(PetCatalog::load(&codex_home));
            (state, Some(store), Some(codex_home), None)
        }
        Err(_) => {
            diagnostics::warn("state", "restore", "invalid_state");
            (
                AppState::with_pet_catalog(PetCatalog::load(&codex_home)),
                None,
                Some(codex_home),
                None,
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
    if spool.recover_claims().is_err() {
        diagnostics::warn("spool", "recover_claims", "recovery_failed");
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
            Err(_) => {
                diagnostics::warn("spool", "claim", "read_failed");
                break;
            }
        };
        match handle.ingest_spooled(claim.event().clone()).await {
            Ok(_) => {
                if claim.commit().is_err() {
                    diagnostics::warn("spool", "commit", "commit_failed");
                    break;
                }
            }
            Err(_) => {
                if claim.release().is_err() {
                    diagnostics::warn("spool", "release", "release_failed");
                }
                diagnostics::warn("ingestion", "reduce_spooled", "unavailable");
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
    let payload = match connection.read_payload().await {
        Ok(payload) => payload,
        Err(_) => {
            let _ = handle.record_transport_rejection().await;
            diagnostics::warn("ingestion", "read_frame", "transport_rejected");
            return;
        }
    };
    let now_ms = unix_time_ms();
    match handle.ingest(payload, now_ms).await {
        Ok(acknowledgement) => {
            if connection
                .write_acknowledgement(&acknowledgement)
                .await
                .is_err()
            {
                diagnostics::warn("ingestion", "write_acknowledgement", "transport_failed");
            }
        }
        Err(_) => diagnostics::warn("ingestion", "verify_message", "message_rejected"),
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
    let size = window.outer_size().map_err(|error| error.to_string())?;
    let displays = display_work_areas(window)?;
    let resolved = resolve_window_placement(
        saved,
        &displays,
        size.width,
        size.height,
        DEFAULT_VISIBLE_WINDOW_MARGIN,
    )
    .ok_or_else(|| "no display work area is available".to_owned())?;
    let current = window.outer_position().map_err(|error| error.to_string())?;
    if current.x != resolved.physical_x() || current.y != resolved.physical_y() {
        window
            .set_position(tauri::PhysicalPosition::new(
                resolved.physical_x(),
                resolved.physical_y(),
            ))
            .map_err(|error| error.to_string())?;
    }
    Ok(resolved.placement().clone())
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

fn setup_tray(app: &tauri::App, state: AppState, codex_home: Option<&Path>) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
    let hide = MenuItem::with_id(app, "hide", "Hide", true, None::<&str>)?;
    let always_on_top = CheckMenuItem::with_id(
        app,
        "always-on-top",
        "Always on Top",
        true,
        tauri::async_runtime::block_on(state.settings()).always_on_top,
        None::<&str>,
    )?;
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
    let integration_status = codex_home.map_or(TrayIntegrationStatus::Unavailable, |codex_home| {
        TrayIntegrationStatus::from_inspection(&inspect(codex_home))
    });
    let integration = MenuItem::with_id(
        app,
        "integration-status",
        integration_status.label(),
        false,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let diagnostics = MenuItem::with_id(app, "diagnostics", "Diagnostics", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &hide,
            &always_on_top,
            &pet_menu,
            &integration,
            &settings,
            &diagnostics,
            &separator,
            &quit,
        ],
    )?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".to_owned()))?;

    TrayIconBuilder::with_id("lili-tray")
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
        .on_menu_event({
            let state = state.clone();
            let codex_home = codex_home.map(Path::to_path_buf);
            move |app, event| match TrayAction::parse(event.id().as_ref()) {
                TrayAction::Show => {
                    if let Some(window) = app.get_webview_window("pet") {
                        clear_tray_view(&window);
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                TrayAction::Hide => {
                    if let Some(window) = app.get_webview_window("pet") {
                        let _ = window.hide();
                    }
                }
                TrayAction::AlwaysOnTop => {
                    let enabled = always_on_top.is_checked().unwrap_or(true);
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
                TrayAction::SelectPet(pet_id) => {
                    let Some(codex_home) = codex_home.as_deref() else {
                        return;
                    };
                    if select_pet(&state, codex_home, &pet_id).is_ok() {
                        for (candidate, item) in &pet_items {
                            let _ = item.set_checked(candidate == &pet_id);
                        }
                        if let Some(window) = app.get_webview_window("pet") {
                            clear_tray_view(&window);
                            let _ = window.show();
                        }
                    }
                }
                TrayAction::Settings => show_tray_view(app, TrayView::Settings),
                TrayAction::Diagnostics => show_tray_view(app, TrayView::Diagnostics),
                TrayAction::Quit => app.exit(0),
                TrayAction::IntegrationStatus | TrayAction::Unknown => {}
            }
        })
        .build(app)?;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TrayAction {
    Show,
    Hide,
    AlwaysOnTop,
    SelectPet(PetId),
    IntegrationStatus,
    Settings,
    Diagnostics,
    Quit,
    Unknown,
}

impl TrayAction {
    fn parse(id: &str) -> Self {
        match id {
            "show" => Self::Show,
            "hide" => Self::Hide,
            "always-on-top" => Self::AlwaysOnTop,
            "integration-status" => Self::IntegrationStatus,
            "settings" => Self::Settings,
            "diagnostics" => Self::Diagnostics,
            "quit" => Self::Quit,
            _ => id
                .strip_prefix("pet:")
                .and_then(PetId::parse)
                .map_or(Self::Unknown, Self::SelectPet),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrayIntegrationStatus {
    Installed,
    Partial,
    NotConfigured,
    NeedsAttention,
    Unavailable,
}

impl TrayIntegrationStatus {
    fn from_inspection(inspection: &lili_integration::IntegrationInspection) -> Self {
        let notify_installed = inspection.notify.kind == IntegrationKind::Lili;
        let installed_hooks = inspection
            .hook_surfaces
            .iter()
            .filter(|surface| surface.lili_handlers > 0)
            .count();
        if notify_installed && installed_hooks == inspection.hook_surfaces.len() {
            Self::Installed
        } else if notify_installed || installed_hooks > 0 {
            Self::Partial
        } else if inspection.notify.kind == IntegrationKind::Missing
            && inspection
                .hook_surfaces
                .iter()
                .all(|surface| surface.other_handlers == 0)
        {
            Self::NotConfigured
        } else {
            Self::NeedsAttention
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Installed => "Integration: Installed",
            Self::Partial => "Integration: Partial",
            Self::NotConfigured => "Integration: Not Configured",
            Self::NeedsAttention => "Integration: Needs Attention",
            Self::Unavailable => "Integration: Unavailable",
        }
    }
}

fn select_pet(state: &AppState, codex_home: &Path, pet_id: &PetId) -> Result<(), String> {
    let catalog = PetCatalog::load_with_selection(codex_home, Some(pet_id));
    if catalog.active().definition().id() != pet_id {
        return Err("selected pet is unavailable".to_owned());
    }
    persist_selected_pet(codex_home, pet_id).map_err(|error| error.to_string())?;
    tauri::async_runtime::block_on(state.replace_pet_catalog(catalog));
    Ok(())
}

#[derive(Clone, Copy)]
enum TrayView {
    Settings,
    Diagnostics,
}

fn show_tray_view(app: &tauri::AppHandle, view: TrayView) {
    let Some(window) = app.get_webview_window("pet") else {
        return;
    };
    let value = match view {
        TrayView::Settings => "settings",
        TrayView::Diagnostics => "diagnostics",
    };
    let _ = window.eval(format!(
        "document.getElementById('lili-app')?.setAttribute('data-tray-view','{value}')"
    ));
    let _ = window.show();
    let _ = window.set_focus();
}

fn clear_tray_view(window: &tauri::WebviewWindow) {
    let _ = window.eval("document.getElementById('lili-app')?.removeAttribute('data-tray-view')");
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

    #[test]
    fn tray_actions_reject_untrusted_pet_identifiers() {
        assert_eq!(TrayAction::parse("show"), TrayAction::Show);
        assert_eq!(TrayAction::parse("always-on-top"), TrayAction::AlwaysOnTop);
        assert_eq!(
            TrayAction::parse("pet:lili"),
            TrayAction::SelectPet(PetId::parse("lili").unwrap())
        );
        assert_eq!(TrayAction::parse("pet:bad\nvalue"), TrayAction::Unknown);
        assert_eq!(TrayAction::parse("unknown"), TrayAction::Unknown);
    }

    #[test]
    fn missing_configuration_reports_not_configured_in_tray() {
        let root =
            std::env::temp_dir().join(format!("lili-tray-integration-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let inspection = lili_integration::inspect_with_version(&root, Some("0.147.0".to_owned()));
        assert_eq!(
            TrayIntegrationStatus::from_inspection(&inspection),
            TrayIntegrationStatus::NotConfigured
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
