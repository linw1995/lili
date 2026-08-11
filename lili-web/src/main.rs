use std::net::{Ipv4Addr, SocketAddr};

use axum::{Json, Router, response::Html, routing::get};
use leptos::prelude::*;
use lili_ui::App;
use tokio::net::TcpListener;
use tower_http::services::ServeDir;
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

    tracing::info!(%address, "fixture web server listening");
    axum::serve(listener, fixture_router())
        .await
        .expect("fixture web server stopped unexpectedly");
}

fn fixture_router() -> Router {
    Router::new()
        .route("/health", get(health))
        .nest_service("/assets", ServeDir::new("dist/assets"))
        .fallback(get(ssr_shell))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn ssr_shell() -> Html<String> {
    let app = view! { <App/> }.to_html();
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><link rel=\"stylesheet\" href=\"/assets/lili.css\"><title>Lili</title></head><body>{app}</body></html>"
    ))
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn health_is_deterministic() {
        let response = fixture_router()
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, r#"{"status":"ok"}"#);
    }

    #[tokio::test]
    async fn shell_contains_ssr_marker() {
        let response = fixture_router()
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("data-ssr-marker=\"lili-ready\""));
    }
}
