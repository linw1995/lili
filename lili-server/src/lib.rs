use std::{collections::HashSet, convert::Infallible, path::PathBuf, sync::Arc, time::Duration};

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
use lili_actions::{ActionAuditEntry, EffectiveActionsView, InteractionTrigger};
use lili_app_state::{
    AppState, AppStateStore, IngestionDiagnostics, NativeIngestionHandle, UserSettings,
};
use lili_core::{DiagnosticPrivacy, PetPresentationState, diagnostic_privacy};
use lili_session::{CodexAdapterDiagnostics, NotificationId, ReductionOutcome};
use lili_ui::App;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc, watch};
use tokio_stream::wrappers::ReceiverStream;
use tower_http::services::{ServeDir, ServeFile};
use uuid::Uuid;

const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;
const MAX_FIXTURE_NOTIFICATIONS: usize = 32;
const MAX_FIXTURE_TEXT_BYTES: usize = 4 * 1024;
const CONTEXT_MENU_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <style>
    :root { color-scheme: light dark; font-family: ui-rounded, system-ui, sans-serif; }
    html, body { margin: 0; background: transparent; }
    menu {
      backdrop-filter: blur(14px);
      background: rgb(24 28 36 / 96%);
      border: 1px solid rgb(255 255 255 / 24%);
      border-radius: 10px;
      box-shadow: 0 10px 28px rgb(0 0 0 / 32%);
      color: #fff;
      display: grid;
      gap: 3px;
      list-style: none;
      margin: 0;
      min-width: 184px;
      padding: 5px;
    }
    button {
      background: transparent;
      border: 0;
      border-radius: 6px;
      color: inherit;
      cursor: pointer;
      font: 500 12px/1.35 system-ui, sans-serif;
      padding: 7px 9px;
      text-align: start;
      width: 100%;
    }
    button:hover, button:focus-visible { background: rgb(120 215 255 / 20%); outline: none; }
  </style>
</head>
<body>
  <menu aria-label="Pet menu" role="menu">
    <button type="button" role="menuitem" data-action="show">Show</button>
    <button type="button" role="menuitem" data-action="hide">Hide</button>
    <button type="button" role="menuitem" data-action="always-on-top">Always on Top</button>
    <button type="button" role="menuitem" data-action="settings">Settings</button>
    <button type="button" role="menuitem" data-action="diagnostics">Diagnostics</button>
    <button type="button" role="menuitem" data-action="quit">Quit</button>
  </menu>
</body>
</html>"#;

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
#[serde(rename_all = "camelCase")]
struct Diagnostics {
    status: &'static str,
    transport: &'static str,
    ingestion: IngestionDiagnostics,
    actions: EffectiveActionsView,
    action_audit: Vec<ActionAuditEntry>,
    privacy: DiagnosticPrivacy,
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

#[derive(Clone)]
struct ServerState {
    app: AppState,
    fixture: Option<FixturePresentationStore>,
    diagnostics_refresh: Option<NativeDiagnosticsRefresh>,
    persistence_store: Option<AppStateStore>,
}

type CodexDiagnosticsInspector = Arc<dyn Fn() -> CodexAdapterDiagnostics + Send + Sync>;

#[derive(Clone)]
pub struct NativeDiagnosticsRefresh {
    ingestion: NativeIngestionHandle,
    inspector: CodexDiagnosticsInspector,
}

impl NativeDiagnosticsRefresh {
    pub fn new(
        ingestion: NativeIngestionHandle,
        inspector: impl Fn() -> CodexAdapterDiagnostics + Send + Sync + 'static,
    ) -> Self {
        Self {
            ingestion,
            inspector: Arc::new(inspector),
        }
    }
}

#[derive(Clone)]
struct FixturePresentationStore {
    presentation: Arc<RwLock<PetPresentationState>>,
    sender: Arc<watch::Sender<PetPresentationState>>,
    approved_asset_id: String,
}

impl FixturePresentationStore {
    fn new(initial: PetPresentationState) -> Self {
        let approved_asset_id = initial.pet_asset_id.clone().unwrap_or_default();
        let (sender, _) = watch::channel(initial.clone());
        Self {
            presentation: Arc::new(RwLock::new(initial)),
            sender: Arc::new(sender),
            approved_asset_id,
        }
    }

    async fn current(&self) -> PetPresentationState {
        self.presentation.read().await.clone()
    }

    fn subscribe(&self) -> watch::Receiver<PetPresentationState> {
        self.sender.subscribe()
    }

    async fn replace(&self, mut next: PetPresentationState) -> Result<(), &'static str> {
        validate_fixture_presentation(&next)?;
        let mut current = self.presentation.write().await;
        next.revision = current.revision.saturating_add(1);
        *current = next.clone();
        self.sender.send_replace(next);
        Ok(())
    }

    async fn replace_reduced_motion(&self, reduced_motion: bool) {
        let mut current = self.presentation.write().await;
        current.revision = current.revision.saturating_add(1);
        current.reduced_motion = reduced_motion;
        self.sender.send_replace(current.clone());
    }

    async fn dismiss(&self, notification_id: &str) -> bool {
        let mut current = self.presentation.write().await;
        let before = current.notifications.len();
        current
            .notifications
            .retain(|notification| notification.activation_id != notification_id);
        if current.notifications.len() == before {
            return false;
        }
        current.unread_notification_count = current.notifications.len();
        current.revision = current.revision.saturating_add(1);
        self.sender.send_replace(current.clone());
        true
    }

    async fn accepts_interaction(&self, request: &InteractionRequest) -> bool {
        match request.trigger.as_str() {
            "pet_click" | "pet_double_click" => request.notification_id.is_none(),
            "notification_click" | "notification_activate" => {
                let Some(notification_id) = request.notification_id.as_deref() else {
                    return false;
                };
                self.presentation
                    .read()
                    .await
                    .notifications
                    .iter()
                    .any(|notification| notification.activation_id == notification_id)
            }
            _ => false,
        }
    }

    async fn serves_asset(&self, asset_id: &str) -> bool {
        self.presentation.read().await.pet_asset_id.as_deref() == Some(asset_id)
    }
}

impl ServerState {
    fn native(app: AppState, diagnostics_refresh: Option<NativeDiagnosticsRefresh>) -> Self {
        Self::native_with_persistence(app, diagnostics_refresh, None)
    }

    fn native_with_persistence(
        app: AppState,
        diagnostics_refresh: Option<NativeDiagnosticsRefresh>,
        persistence_store: Option<AppStateStore>,
    ) -> Self {
        Self {
            app,
            fixture: None,
            diagnostics_refresh,
            persistence_store,
        }
    }

    fn fixture(app: AppState) -> Self {
        let initial = app.subscribe_pet_presentation().borrow().clone();
        Self {
            app,
            fixture: Some(FixturePresentationStore::new(initial)),
            diagnostics_refresh: None,
            persistence_store: None,
        }
    }

    async fn ingestion_diagnostics(&self) -> IngestionDiagnostics {
        let Some(refresh) = self.diagnostics_refresh.clone() else {
            return self.app.ingestion_diagnostics().await;
        };
        let inspector = Arc::clone(&refresh.inspector);
        let Ok(discovered) = tokio::task::spawn_blocking(move || inspector()).await else {
            return self.app.ingestion_diagnostics().await;
        };
        match refresh.ingestion.refresh_codex_adapter(discovered).await {
            Ok(diagnostics) => diagnostics,
            Err(_) => self.app.ingestion_diagnostics().await,
        }
    }

    async fn presentation(&self) -> PetPresentationState {
        match &self.fixture {
            Some(fixture) => fixture.current().await,
            None => self.app.pet_presentation().await,
        }
    }

    fn subscribe_presentation(&self) -> watch::Receiver<PetPresentationState> {
        match &self.fixture {
            Some(fixture) => fixture.subscribe(),
            None => self.app.subscribe_pet_presentation(),
        }
    }
}

fn validate_fixture_presentation(presentation: &PetPresentationState) -> Result<(), &'static str> {
    if presentation.notifications.len() > MAX_FIXTURE_NOTIFICATIONS
        || presentation.unread_notification_count != presentation.notifications.len()
        || presentation.pet_label.len() > MAX_FIXTURE_TEXT_BYTES
        || presentation
            .action_feedback
            .as_ref()
            .is_some_and(|feedback| {
                feedback.action_id.len() > MAX_FIXTURE_TEXT_BYTES
                    || feedback.message.len() > MAX_FIXTURE_TEXT_BYTES
            })
    {
        return Err("fixture presentation exceeds its bounds");
    }
    let mut notification_ids = HashSet::new();
    for notification in &presentation.notifications {
        if notification.activation_id.is_empty()
            || notification.activation_id.len() > MAX_FIXTURE_TEXT_BYTES
            || notification.summary.len() > MAX_FIXTURE_TEXT_BYTES
            || notification
                .project_label
                .as_ref()
                .is_some_and(|label| label.len() > MAX_FIXTURE_TEXT_BYTES)
            || !notification_ids.insert(&notification.activation_id)
        {
            return Err("fixture notification is invalid");
        }
    }
    Ok(())
}

pub fn build_native_router(state: AppState, assets: Option<StaticAssets>) -> Router {
    build_router(state, assets)
}

pub fn build_native_router_with_diagnostics(
    state: AppState,
    assets: Option<StaticAssets>,
    diagnostics_refresh: Option<NativeDiagnosticsRefresh>,
) -> Router {
    build_router_with_diagnostics(state, assets, diagnostics_refresh)
}

pub fn build_native_router_with_diagnostics_and_persistence(
    state: AppState,
    assets: Option<StaticAssets>,
    diagnostics_refresh: Option<NativeDiagnosticsRefresh>,
    persistence_store: Option<AppStateStore>,
) -> Router {
    build_server_router(
        ServerState::native_with_persistence(state, diagnostics_refresh, persistence_store),
        assets,
        false,
    )
}

pub fn build_fixture_router(assets: Option<StaticAssets>) -> Router {
    build_server_router(ServerState::fixture(AppState::default()), assets, true)
}

fn build_router(state: AppState, assets: Option<StaticAssets>) -> Router {
    build_router_with_diagnostics(state, assets, None)
}

fn build_router_with_diagnostics(
    state: AppState,
    assets: Option<StaticAssets>,
    diagnostics_refresh: Option<NativeDiagnosticsRefresh>,
) -> Router {
    build_server_router(
        ServerState::native(state, diagnostics_refresh),
        assets,
        false,
    )
}

fn build_server_router(state: ServerState, assets: Option<StaticAssets>, fixture: bool) -> Router {
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
        .route("/context-menu", get(context_menu))
        .route("/pet-assets/{asset_id}", get(pet_asset))
        .route("/presentation-events", get(events))
        .nest("/api/v1", api)
        .fallback(get(ssr_shell));

    if fixture {
        router = router.route("/__fixture/presentation", put(update_fixture_presentation));
    }

    let mut router = router
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES))
        .layer(middleware::from_fn(security_headers))
        .with_state(state);

    if let Some(assets) = assets {
        let index = assets.root.join("index.html");
        router = router
            .route_service(
                "/lili-ui.js",
                ServeFile::new(assets.root.join("lili-ui.js")),
            )
            .route_service(
                "/lili-ui_bg.wasm",
                ServeFile::new(assets.root.join("lili-ui_bg.wasm")),
            )
            .nest_service("/snippets", ServeDir::new(assets.root.join("snippets")));
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

async fn context_menu() -> Html<&'static str> {
    Html(CONTEXT_MENU_HTML)
}

async fn snapshot(State(state): State<ServerState>) -> Json<lili_app_state::ViewSnapshot> {
    Json(state.app.snapshot().await)
}

async fn settings(State(state): State<ServerState>) -> Json<UserSettings> {
    Json(state.app.settings().await)
}

async fn update_settings(
    State(state): State<ServerState>,
    Json(settings): Json<UserSettings>,
) -> Json<UserSettings> {
    let updated = state.app.replace_settings(settings).await;
    if let Some(fixture) = &state.fixture {
        fixture.replace_reduced_motion(updated.reduced_motion).await;
    }
    Json(updated)
}

async fn diagnostics(State(state): State<ServerState>) -> Json<Diagnostics> {
    let (ingestion, actions, action_audit) = tokio::join!(
        state.ingestion_diagnostics(),
        state.app.effective_actions(),
        state.app.action_audit(),
    );
    Json(Diagnostics {
        status: "ok",
        transport: "fixture",
        ingestion,
        actions,
        action_audit,
        privacy: diagnostic_privacy(),
    })
}

async fn interaction(
    State(state): State<ServerState>,
    Json(request): Json<InteractionRequest>,
) -> Json<InteractionResponse> {
    let request_id = Uuid::new_v4();
    if let Some(fixture) = &state.fixture {
        return Json(InteractionResponse {
            accepted: fixture.accepts_interaction(&request).await,
            request_id,
        });
    }
    let binding = match request.trigger.as_str() {
        "notification_click" | "notification_activate" => {
            let notification_id = request
                .notification_id
                .and_then(|value| NotificationId::parse(value).ok());
            state
                .app
                .bind_interaction(
                    request_id,
                    unix_time_ms(),
                    InteractionTrigger::NotificationActivate,
                    notification_id.as_ref(),
                )
                .await
        }
        "pet_click" if request.notification_id.is_none() => {
            state
                .app
                .bind_interaction(
                    request_id,
                    unix_time_ms(),
                    InteractionTrigger::PetClick,
                    None,
                )
                .await
        }
        "pet_double_click" if request.notification_id.is_none() => {
            state
                .app
                .bind_interaction(
                    request_id,
                    unix_time_ms(),
                    InteractionTrigger::PetDoubleClick,
                    None,
                )
                .await
        }
        _ => None,
    };
    let accepted = match binding {
        Some(context) => state.app.dispatch_interaction(context).await.accepted,
        None => false,
    };
    Json(InteractionResponse {
        accepted,
        request_id,
    })
}

async fn dismiss_notification(
    State(state): State<ServerState>,
    Path(notification_id): Path<String>,
) -> (StatusCode, Json<NotificationMutationResponse>) {
    if let Some(fixture) = &state.fixture {
        let accepted = fixture.dismiss(&notification_id).await;
        return (
            if accepted {
                StatusCode::OK
            } else {
                StatusCode::NOT_FOUND
            },
            Json(NotificationMutationResponse { accepted }),
        );
    }
    let Ok(notification_id) = NotificationId::parse(notification_id) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(NotificationMutationResponse { accepted: false }),
        );
    };
    let outcome = if let Some(store) = &state.persistence_store {
        match state
            .app
            .acknowledge_notification_persisted(&notification_id, unix_time_ms(), store)
            .await
        {
            Ok(outcome) => outcome,
            Err(_) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(NotificationMutationResponse { accepted: false }),
                );
            }
        }
    } else {
        state
            .app
            .acknowledge_notification(&notification_id, unix_time_ms())
            .await
    };
    let accepted = matches!(outcome, ReductionOutcome::Applied { .. });
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

async fn pet_asset(State(state): State<ServerState>, Path(asset_id): Path<String>) -> Response {
    let approved_asset_id = match &state.fixture {
        Some(fixture) if fixture.serves_asset(&asset_id).await => &fixture.approved_asset_id,
        _ => &asset_id,
    };
    let Some(asset) = state.app.approved_pet_asset(approved_asset_id).await else {
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
    State(state): State<ServerState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let mut presentations = state.subscribe_presentation();
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

async fn ssr_shell(State(state): State<ServerState>) -> Html<String> {
    let presentation = state.presentation().await;
    let app = view! { <App presentation/> }.to_html();
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><link rel=\"stylesheet\" href=\"/assets/lili.css\"><script type=\"module\" src=\"/assets/lili-bootstrap.js\"></script><title>Lili</title></head><body>{app}</body></html>"
    ))
}

async fn update_fixture_presentation(
    State(state): State<ServerState>,
    Json(presentation): Json<PetPresentationState>,
) -> (StatusCode, Json<PetPresentationState>) {
    let Some(fixture) = &state.fixture else {
        return (StatusCode::NOT_FOUND, Json(presentation));
    };
    match fixture.replace(presentation).await {
        Ok(()) => (StatusCode::OK, Json(fixture.current().await)),
        Err(_) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(fixture.current().await),
        ),
    }
}

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; base-uri 'none'; connect-src 'self'; font-src 'self'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data:; object-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'",
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
    use lili_session::{
        CodexPluginAvailability, CodexPluginDiagnostics, ForwardingCredentials,
        ProviderCapabilitiesInputV1, ProviderInputV1, TESTED_CODEX_VERSION,
        normalize_provider_input,
    };
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
    async fn context_menu_contains_only_native_actions() {
        let response = build_router(AppState::default(), None)
            .oneshot(Request::get("/context-menu").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        for action in [
            "show",
            "hide",
            "always-on-top",
            "settings",
            "diagnostics",
            "quit",
        ] {
            assert!(body.contains(&format!("data-action=\"{action}\"")));
        }
        assert!(!body.contains("invoke("));
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
        assert_eq!(diagnostics["privacy"]["schemaVersion"], 1);
        assert_eq!(diagnostics["privacy"]["contentPolicy"], "metadata_only");
        assert!(
            diagnostics["privacy"]["excludedFields"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("mac_secret"))
        );
    }

    #[tokio::test]
    async fn diagnostics_refresh_plugin_discovery_without_desktop_restart() {
        let state = AppState::default();
        let credentials = ForwardingCredentials::generate().unwrap();
        let (handle, actor) =
            lili_app_state::NativeIngestionActor::channel(state.clone(), credentials, 1).await;
        let task = tokio::spawn(actor.run());
        let current = Arc::new(std::sync::Mutex::new(
            CodexAdapterDiagnostics::with_discovery(Some(TESTED_CODEX_VERSION), []).with_plugin(
                CodexPluginDiagnostics::discovered(
                    Some(TESTED_CODEX_VERSION),
                    CodexPluginAvailability::Installed,
                    true,
                    true,
                    Some(env!("CARGO_PKG_VERSION")),
                    false,
                )
                .with_plugin_id(Some("lili@lili-local")),
            ),
        ));
        let inspected = Arc::clone(&current);
        let refresh = NativeDiagnosticsRefresh::new(handle.clone(), move || {
            inspected.lock().unwrap().clone()
        });
        let router = build_router_with_diagnostics(state, None, Some(refresh));

        let installed = router
            .clone()
            .oneshot(
                Request::get("/api/v1/diagnostics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(installed.into_body(), usize::MAX).await.unwrap();
        let diagnostics: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            diagnostics["ingestion"]["codexAdapter"]["plugin"]["installed"],
            true
        );

        *current.lock().unwrap() =
            CodexAdapterDiagnostics::with_discovery(Some(TESTED_CODEX_VERSION), []).with_plugin(
                CodexPluginDiagnostics::discovered(
                    Some(TESTED_CODEX_VERSION),
                    CodexPluginAvailability::Available,
                    false,
                    false,
                    Some(env!("CARGO_PKG_VERSION")),
                    false,
                )
                .with_plugin_id(Some("lili@lili-local")),
            );
        let removed = router
            .oneshot(
                Request::get("/api/v1/diagnostics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(removed.into_body(), usize::MAX).await.unwrap();
        let diagnostics: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            diagnostics["ingestion"]["codexAdapter"]["plugin"]["installed"],
            false
        );
        assert_eq!(
            diagnostics["ingestion"]["codexAdapter"]["plugin"]["availability"],
            "available"
        );

        drop(handle);
        task.await.unwrap();
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
    async fn fixture_router_replaces_only_bounded_presentation_state() {
        let router = build_fixture_router(None);
        let presentation = PetPresentationState {
            pet_asset_id: Some("selected-fixture-pet".to_owned()),
            pet_label: "Selected fixture pet".to_owned(),
            ..PetPresentationState::default()
        };
        let response = router
            .clone()
            .oneshot(
                Request::put("/__fixture/presentation")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&presentation).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response = router
            .clone()
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("Selected fixture pet"));
        assert!(body.contains("/pet-assets/selected-fixture-pet"));

        let response = router
            .oneshot(
                Request::get("/pet-assets/selected-fixture-pet")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn native_router_does_not_expose_fixture_control() {
        let response = build_router(AppState::default(), None)
            .oneshot(
                Request::put("/__fixture/presentation")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
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

    #[cfg(unix)]
    #[tokio::test]
    async fn interaction_endpoint_dispatches_native_action_and_exposes_redacted_audit() {
        let (state, notification_id) = state_with_notification().await;
        let loaded = lili_actions::load_actions_str(
            r#"
version = 1

[[action]]
id = "open-session"
trigger = "notification_activate"
command = ["/bin/sh", "-c", "printf private-output"]
debounce_ms = 0
"#,
            &lili_actions::ActionLoadContext::new("/", "/", Vec::new()),
        );
        assert!(state.configure_actions(loaded, 1).await);
        let router = build_router(state.clone(), None);
        let activation = serde_json::json!({
            "trigger": "notification_click",
            "notification_id": notification_id,
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
        for _ in 0..100 {
            if !state.action_audit().await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(state.action_audit().await.len(), 1);
        assert_eq!(state.pet_presentation().await.unread_notification_count, 1);

        let response = router
            .oneshot(
                Request::get("/api/v1/diagnostics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let diagnostics: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(diagnostics["actions"]["actions"][0]["id"], "open-session");
        assert_eq!(diagnostics["actionAudit"][0]["eventId"], "event-card");
        let serialized = String::from_utf8(body.to_vec()).unwrap();
        assert!(!serialized.contains("private-output"));
        assert!(!serialized.contains("/bin/sh"));
    }

    #[tokio::test]
    async fn failure_injection_during_webview_reload_replays_latest_snapshot() {
        let state = AppState::default();
        let router = build_router(state.clone(), None);
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
        drop(body);

        state
            .replace_settings(UserSettings {
                always_on_top: false,
                reduced_motion: true,
            })
            .await;
        let response = router
            .oneshot(
                Request::get("/presentation-events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let mut body = response.into_body().into_data_stream();
        let reloaded = tokio::time::timeout(Duration::from_secs(1), body.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let reloaded = String::from_utf8(reloaded.to_vec()).unwrap();
        assert!(reloaded.starts_with("event: snapshot\n"));
        assert!(reloaded.contains("id: 1\n"));
        assert_eq!(reloaded.matches("event: snapshot").count(), 1);
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
        let package_dir = temp.0.join("pets").join("lili");
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
