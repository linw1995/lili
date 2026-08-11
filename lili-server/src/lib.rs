use std::{convert::Infallible, path::PathBuf, time::Duration};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderValue, Request, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response, Sse, sse::Event, sse::KeepAlive},
    routing::{get, post, put},
};
use leptos::prelude::*;
use lili_app_state::{AppState, IngestionDiagnostics, UserSettings};
use lili_session::{NotificationId, ReductionOutcome};
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
    ingestion: IngestionDiagnostics,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct NotificationMutationResponse {
    accepted: bool,
}

pub fn build_router(state: AppState, assets: Option<StaticAssets>) -> Router {
    let api = Router::new()
        .route("/snapshot", get(snapshot))
        .route("/events", get(events))
        .route("/settings", get(settings).merge(put(update_settings)))
        .route("/interactions", post(interaction))
        .route(
            "/notifications/{notification_id}/dismiss",
            post(dismiss_notification),
        )
        .route("/diagnostics", get(diagnostics));

    let mut router = Router::new()
        .route("/health", get(health))
        .route("/pet-assets/{asset_id}", get(pet_asset))
        .route("/presentation-events", get(events))
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

async fn diagnostics(State(state): State<AppState>) -> Json<Diagnostics> {
    Json(Diagnostics {
        status: "ok",
        transport: "fixture",
        ingestion: state.ingestion_diagnostics().await,
    })
}

async fn interaction(
    State(state): State<AppState>,
    Json(request): Json<InteractionRequest>,
) -> Json<InteractionResponse> {
    let accepted = if let Some(notification_id) = request.notification_id {
        if let Ok(notification_id) = NotificationId::parse(notification_id) {
            request.trigger == "notification_click"
                && state.notification_context(&notification_id).await.is_some()
        } else {
            false
        }
    } else {
        !request.trigger.is_empty()
    };
    Json(InteractionResponse {
        accepted,
        request_id: Uuid::new_v4(),
    })
}

async fn dismiss_notification(
    State(state): State<AppState>,
    Path(notification_id): Path<String>,
) -> (StatusCode, Json<NotificationMutationResponse>) {
    let Ok(notification_id) = NotificationId::parse(notification_id) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(NotificationMutationResponse { accepted: false }),
        );
    };
    let accepted = matches!(
        state
            .acknowledge_notification(&notification_id, unix_time_ms())
            .await,
        ReductionOutcome::Applied { .. }
    );
    (
        if accepted {
            StatusCode::OK
        } else {
            StatusCode::NOT_FOUND
        },
        Json(NotificationMutationResponse { accepted }),
    )
}

fn unix_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

async fn pet_asset(State(state): State<AppState>, Path(asset_id): Path<String>) -> Response {
    let Some(asset) = state.approved_pet_asset(&asset_id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    Response::builder()
        .header(header::CONTENT_TYPE, asset.content_type())
        .header(
            header::CACHE_CONTROL,
            "private, max-age=31536000, immutable",
        )
        .body(Body::from(asset.bytes().to_vec()))
        .expect("approved pet asset response must be valid")
}

async fn events(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let mut presentations = state.subscribe_pet_presentation();
    let snapshot = presentations.borrow_and_update().clone();
    let (sender, receiver) = mpsc::channel(8);
    tokio::spawn(async move {
        let mut last_revision = snapshot.revision;
        if sender
            .send(Ok(presentation_event("snapshot", &snapshot)))
            .await
            .is_err()
        {
            return;
        }
        while presentations.changed().await.is_ok() {
            let presentation = presentations.borrow_and_update().clone();
            if presentation.revision <= last_revision {
                continue;
            }
            last_revision = presentation.revision;
            if sender
                .send(Ok(presentation_event("presentation", &presentation)))
                .await
                .is_err()
            {
                break;
            }
        }
    });
    Sse::new(ReceiverStream::new(receiver)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

fn presentation_event(
    event: &'static str,
    presentation: &lili_core::PetPresentationState,
) -> Event {
    Event::default()
        .event(event)
        .id(presentation.revision.to_string())
        .data(serde_json::to_string(presentation).expect("pet presentations must serialize"))
}

async fn ssr_shell(State(state): State<AppState>) -> Html<String> {
    let presentation = state.pet_presentation().await;
    let app = view! { <App presentation/> }.to_html();
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
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use axum::body::to_bytes;
    use http::Request;
    use lili_session::{ProviderCapabilitiesInputV1, ProviderInputV1, normalize_provider_input};
    use tokio_stream::StreamExt;
    use tower::ServiceExt;

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "lili-server-pet-asset-{}-{sequence}",
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

    async fn state_with_notification() -> (AppState, String) {
        let state = AppState::default();
        let event = normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some("turn_completed".to_owned()),
            event_id: Some("event-card".to_owned()),
            session_id: Some("session-card".to_owned()),
            turn_id: Some("turn-card".to_owned()),
            occurred_at_ms: Some(10),
            project: None,
            summary: Some("Done".to_owned()),
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap();
        state.apply_session_event(event).await;
        let notification_id = state.pet_presentation().await.notifications[0]
            .activation_id
            .clone();
        (state, notification_id)
    }

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
    async fn diagnostics_expose_honest_adapter_compatibility() {
        let response = build_router(AppState::default(), None)
            .oneshot(
                Request::get("/api/v1/diagnostics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let diagnostics: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let adapter = &diagnostics["ingestion"]["codexAdapter"];
        assert_eq!(adapter["testedCodexVersion"], "0.147.0");
        assert!(adapter["codexVersion"].is_null());
        assert_eq!(adapter["discoveredSurfaces"], serde_json::json!([]));
        assert!(
            adapter["missingLifecycleCoverage"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("failure"))
        );
        assert!(!adapter["remediation"].as_array().unwrap().is_empty());
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
    async fn notification_activation_and_dismissal_are_independent() {
        let (state, notification_id) = state_with_notification().await;
        let router = build_router(state.clone(), None);
        let activation = serde_json::json!({
            "trigger": "notification_click",
            "notification_id": notification_id.clone(),
        });
        let response = router
            .clone()
            .oneshot(
                Request::post("/api/v1/interactions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(activation.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(serde_json::from_slice::<serde_json::Value>(&body).unwrap()["accepted"] == true);
        assert_eq!(state.pet_presentation().await.unread_notification_count, 1);

        let response = router
            .clone()
            .oneshot(
                Request::post(format!("/api/v1/notifications/{notification_id}/dismiss"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(state.pet_presentation().await.unread_notification_count, 0);

        let response = router
            .oneshot(
                Request::post(format!("/api/v1/notifications/{notification_id}/dismiss"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn presentation_stream_is_snapshot_first_on_every_webview_load() {
        let router = build_router(AppState::default(), None);
        for _ in 0..2 {
            let response = router
                .clone()
                .oneshot(
                    Request::get("/presentation-events")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(header::CONTENT_TYPE),
                Some(&HeaderValue::from_static("text/event-stream"))
            );
            let mut body = response.into_body().into_data_stream();
            let first = tokio::time::timeout(Duration::from_secs(1), body.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let first = String::from_utf8(first.to_vec()).unwrap();
            assert!(first.starts_with("event: snapshot\n"));
            assert!(first.contains("id: 0\n"));
            assert_eq!(first.matches("event: snapshot").count(), 1);
        }
    }

    #[tokio::test]
    async fn shell_renders_the_approved_pet_asset_identity() {
        let state = AppState::default();
        let asset_id = state.snapshot().await.pet_asset_id.unwrap();
        let response = build_router(state, None)
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("class=\"pet-sprite\""));
        assert!(body.contains("class=\"pet-atlas\""));
        assert!(body.contains(&format!("/pet-assets/{asset_id}")));
        assert!(!body.contains("/api/v1/pet-assets/"));
        assert!(!body.contains("spritesheet.webp"));
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
    async fn approved_pet_asset_uses_opaque_identity_and_cache_headers() {
        let state = AppState::default();
        let snapshot = state.snapshot().await;
        let asset_id = snapshot.pet_asset_id.unwrap();
        assert_eq!(asset_id.len(), 32);
        assert!(asset_id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(!asset_id.contains('/'));

        let response = build_router(state, None)
            .oneshot(
                Request::get(format!("/pet-assets/{asset_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE),
            Some(&HeaderValue::from_static("image/webp"))
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static(
                "private, max-age=31536000, immutable"
            ))
        );
        assert!(
            response
                .headers()
                .contains_key(header::CONTENT_SECURITY_POLICY)
        );
        assert_eq!(
            response.headers().get(header::X_CONTENT_TYPE_OPTIONS),
            Some(&HeaderValue::from_static("nosniff"))
        );
        let body = to_bytes(response.into_body(), 32 * 1024 * 1024)
            .await
            .unwrap();
        assert!(body.starts_with(b"RIFF"));
    }

    #[tokio::test]
    async fn unknown_pet_asset_identity_is_rejected() {
        let response = build_router(AppState::default(), None)
            .oneshot(
                Request::get("/pet-assets/not-approved")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn package_reload_invalidates_the_previous_asset_identity() {
        let temp = TempDir::new();
        let package_dir = temp.0.join("pet").join("lili");
        fs::create_dir_all(&package_dir).unwrap();
        fs::write(
            package_dir.join("pet.json"),
            r#"{"id":"lili","displayName":"Lili","description":"Fixture","spriteVersionNumber":2,"spritesheetPath":"spritesheet.webp"}"#,
        )
        .unwrap();
        let fallback_state = AppState::default();
        let fallback_id = fallback_state
            .snapshot()
            .await
            .pet_asset_id
            .expect("fallback must expose an asset identity");
        let fallback = fallback_state
            .approved_pet_asset(&fallback_id)
            .await
            .expect("fallback identity must resolve");
        fs::write(package_dir.join("spritesheet.webp"), fallback.bytes()).unwrap();

        let first = AppState::with_pet_catalog(lili_pet::PetCatalog::load(&temp.0));
        let old_id = first.snapshot().await.pet_asset_id.unwrap();
        fs::write(package_dir.join("spritesheet.webp"), fallback.bytes()).unwrap();
        let reloaded = AppState::with_pet_catalog(lili_pet::PetCatalog::load(&temp.0));
        let new_id = reloaded.snapshot().await.pet_asset_id.unwrap();
        assert_ne!(old_id, new_id);

        let old_response = build_router(reloaded.clone(), None)
            .oneshot(
                Request::get(format!("/pet-assets/{old_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(old_response.status(), StatusCode::NOT_FOUND);

        let new_response = build_router(reloaded, None)
            .oneshot(
                Request::get(format!("/pet-assets/{new_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(new_response.status(), StatusCode::OK);
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
