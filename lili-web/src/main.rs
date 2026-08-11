use std::net::{Ipv4Addr, SocketAddr};

use lili_server::{StaticAssets, build_fixture_router};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, 3746));
    let listener = TcpListener::bind(address)
        .await
        .expect("failed to bind fixture web server");
    let router = build_fixture_router(Some(StaticAssets::new("dist")));

    tracing::info!(%address, "fixture web server listening");
    axum::serve(listener, router)
        .await
        .expect("fixture web server stopped unexpectedly");
}
