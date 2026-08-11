use std::{convert::Infallible, path::PathBuf};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderValue, Request, StatusCode, header},
    middleware::{self, Next},
    response::{Html, Response, Sse, sse::Event},
    routing::{get, post, put},
};
use leptos::prelude::*;
use lili_app_state::{AppState, UserSettings};
use lili_ui::App;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;

const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticAssets {
    root: PathBuf,
}

impl StaticAssets {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct Health {
    status: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct Diagnostics {
    status: &'static str,
    transport: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct InteractionRequest {
    trigger: String,
    notification_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct InteractionResponse {
    accepted: bool,
    request_id: Uuid,
}

pub fn build_router(state: AppState, assets: Option<StaticAssets>) -> Router {
    let api = Router::new()
        .route("/snapshot", get(snapshot))
        .route("/events", get(events))
        .route("/pet-assets/{asset_id}", get(pet_asset))
        .route("/settings", get(settings).merge(put(update_settings)))
        .route("/interactions", post(interaction))
        .route("/diagnostics", get(diagnostics));

    let mut router = Router::new()
        .route("/health", get(health))
        .nest("/api/v1", api)
        .fallback(get(ssr_shell))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .layer(middleware::from_fn(security_headers))
        .with_state(state);

    if let Some(assets) = assets {
        let index = assets.root.join("index.html");
        router = router.nest_service(
            "/assets",
            ServeDir::new(assets.root.join("assets")).fallback(ServeFile::new(index)),
        );
    }

    router
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

async fn snapshot(State(state): State<AppState>) -> Json<lili_app_state::ViewSnapshot> {
    Json(state.snapshot().await)
}

async fn settings(State(state): State<AppState>) -> Json<UserSettings> {
    Json(state.settings().await)
}

async fn update_settings(
    State(state): State<AppState>,
    Json(settings): Json<UserSettings>,
) -> Json<UserSettings> {
    Json(state.replace_settings(settings).await)
}

async fn diagnostics() -> Json<Diagnostics> {
    Json(Diagnostics {
        status: "ok",
        transport: "fixture",
    })
}

async fn interaction(Json(request): Json<InteractionRequest>) -> Json<InteractionResponse> {
    let accepted = !request.trigger.is_empty()
        && request
            .notification_id
            .as_ref()
            .is_none_or(|value| !value.is_empty());
    Json(InteractionResponse {
        accepted,
        request_id: Uuid::new_v4(),
    })
}

async fn pet_asset(Path(_asset_id): Path<String>) -> StatusCode {
    StatusCode::NOT_FOUND
}

async fn events(State(state): State<AppState>) -> Sse<ReceiverStream<Result<Event, Infallible>>> {
    let snapshot = state.snapshot().await;
    let data = serde_json::to_string(&snapshot).expect("view snapshots must serialize");
    let (sender, receiver) = mpsc::channel(1);
    sender
        .send(Ok(Event::default().event("snapshot").data(data)))
        .await
        .expect("new event receiver must remain open");
    Sse::new(ReceiverStream::new(receiver))
}

async fn ssr_shell() -> Html<String> {
    let app = view! { <App/> }.to_html();
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><link rel=\"stylesheet\" href=\"/assets/lili.css\"><title>Lili</title></head><body>{app}</body></html>"
    ))
}

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; base-uri 'none'; connect-src 'self'; font-src 'self'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data:; object-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use http::Request;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn health_is_deterministic() {
        let response = build_router(AppState::default(), None)
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body, r#"{"status":"ok"}"#);
    }

    #[tokio::test]
    async fn shell_contains_ssr_marker() {
        let response = build_router(AppState::default(), None)
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("data-ssr-marker=\"lili-ready\""));
    }

    #[tokio::test]
    async fn security_headers_are_applied_to_api_responses() {
        let response = build_router(AppState::default(), None)
            .oneshot(
                Request::get("/api/v1/snapshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.headers().get(header::X_CONTENT_TYPE_OPTIONS),
            Some(&HeaderValue::from_static("nosniff"))
        );
    }

    #[tokio::test]
    async fn oversized_json_body_is_rejected() {
        let payload = serde_json::json!({
            "trigger": "x".repeat(MAX_REQUEST_BODY_BYTES),
            "notification_id": null,
        })
        .to_string();
        let response = build_router(AppState::default(), None)
            .oneshot(
                Request::post("/api/v1/interactions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
