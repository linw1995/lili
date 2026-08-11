mod desktop_smoke;
mod ipc_signer;
mod loopback;
mod platform_pinning;

use std::path::{Path, PathBuf};

use desktop_smoke::{DesktopSmokeState, complete_desktop_smoke};
use ipc_signer::{FETCH_SIGNER_SCRIPT, sign_loopback_request};
use lili_app_state::AppState;
use lili_server::{StaticAssets, build_router};
use loopback::LoopbackServer;
use tauri::{
    Manager, WebviewUrl, WebviewWindowBuilder,
    ipc::CapabilityBuilder,
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
};
use tokio::sync::oneshot;

pub fn run() {
    let smoke = std::env::args().any(|argument| argument == "--desktop-smoke");
    run_desktop(smoke);
}

fn run_desktop(smoke: bool) {
    let app = tauri::Builder::default()
        .manage(DesktopSmokeState::default())
        .invoke_handler(tauri::generate_handler![
            sign_loopback_request,
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
    let loopback = LoopbackServer::bind(build_router(AppState::default(), assets))
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
            .permission("allow-sign-loopback-request");
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
    app.run(move |_app, event| {
        if matches!(event, tauri::RunEvent::Exit)
            && let Some(shutdown_tx) = shutdown_tx.take()
        {
            let _ = shutdown_tx.send(());
        }
    });
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
