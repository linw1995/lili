use std::{fs, path::Path};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use lili_server::build_fixture_router;
use tower::ServiceExt;

const FORBIDDEN_DEPENDENCIES: &[&str] = &[
    "lili-actions",
    "lili-app-state",
    "lili-integration",
    "lili-pet",
    "lili-session",
    "tauri",
];

const FORBIDDEN_SOURCE_MARKERS: &[&str] = &[
    "build_native_router",
    "configure_actions",
    "dispatch_interaction",
    "ForwardingCredential",
    "lili_actions",
    "lili_app_state",
    "lili_integration",
    "lili_pet",
    "lili_session",
    "std::fs",
    "std::process",
    "tokio::process",
];

#[test]
fn web_build_has_no_native_capability_dependencies_or_source_access() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(manifest_dir.join("Cargo.toml")).unwrap();
    for dependency in FORBIDDEN_DEPENDENCIES {
        assert!(
            !manifest.lines().any(|line| {
                line.split_once('=')
                    .is_some_and(|(name, _)| name.trim() == *dependency)
            }),
            "forbidden direct dependency: {dependency}"
        );
    }

    let source = read_rust_sources(&manifest_dir.join("src"));
    assert!(source.contains("build_fixture_router"));
    for marker in FORBIDDEN_SOURCE_MARKERS {
        assert!(
            !source.contains(marker),
            "forbidden source access: {marker}"
        );
    }
}

fn read_rust_sources(directory: &Path) -> String {
    let mut source = String::new();
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            source.push_str(&read_rust_sources(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            source.push_str(&fs::read_to_string(path).unwrap());
        }
    }
    source
}

#[tokio::test]
async fn fixture_router_does_not_expose_native_authority() {
    let router = build_fixture_router(None);
    for path in [
        "/api/v1/actions/config",
        "/api/v1/codex/config",
        "/api/v1/credentials",
        "/api/v1/integration/install",
        "/api/v1/pets/path",
        "/api/v1/processes/spawn",
    ] {
        let response = router
            .clone()
            .oneshot(Request::post(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(!response.status().is_success(), "privileged route: {path}");
    }

    let response = router
        .clone()
        .oneshot(
            Request::post("/api/v1/interactions")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"trigger":"pet_click"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

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
    assert_eq!(diagnostics["actions"]["actions"], serde_json::json!([]));
    assert_eq!(diagnostics["actionAudit"], serde_json::json!([]));
}
