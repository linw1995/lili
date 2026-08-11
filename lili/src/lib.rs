mod ipc_signer;
mod loopback;
mod platform_pinning;

use ipc_signer::{FETCH_SIGNER_SCRIPT, sign_loopback_request};
use lili_app_state::AppState;
use lili_server::{StaticAssets, build_router};
use loopback::LoopbackServer;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder, ipc::CapabilityBuilder};
use tokio::sync::oneshot;

pub fn run() {
    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![sign_loopback_request])
        .build(tauri::generate_context!())
        .expect("failed to build Lili");

    let assets = app
        .path()
        .resource_dir()
        .ok()
        .map(|root| StaticAssets::new(root.join("web")))
        .or_else(|| Some(StaticAssets::new("dist")));
    let loopback = LoopbackServer::bind(build_router(AppState::default(), assets))
        .expect("failed to bind secure loopback transport");
    let bootstrap_url = loopback.bootstrap_url();
    let certificate_sha256 = loopback.certificate_sha256();
    let origin = loopback.origin();
    let signer = loopback.signer();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    loopback.spawn(shutdown_rx);

    app.manage(signer);
    app.add_capability(
        CapabilityBuilder::new("loopback-request-signer")
            .remote(format!("{}/*", origin.as_str().trim_end_matches('/')))
            .local(false)
            .window("pet")
            .permission("allow-sign-loopback-request"),
    )
    .expect("failed to register loopback signer capability");

    let allowed_origin = origin.origin();
    let window = WebviewWindowBuilder::new(
        app.handle(),
        "pet",
        WebviewUrl::External("about:blank".parse().expect("valid bootstrap URL")),
    )
    .initialization_script(FETCH_SIGNER_SCRIPT)
    .title("Lili")
    .on_navigation(move |url| url.origin() == allowed_origin)
    .build()
    .expect("failed to create pet window");
    platform_pinning::install_and_navigate(&window, bootstrap_url, certificate_sha256)
        .expect("failed to install loopback certificate pinning");

    let mut shutdown_tx = Some(shutdown_tx);
    app.run(move |_app, event| {
        if matches!(event, tauri::RunEvent::Exit)
            && let Some(shutdown_tx) = shutdown_tx.take()
        {
            let _ = shutdown_tx.send(());
        }
    });
}
