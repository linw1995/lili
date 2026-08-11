use serde_json::Value;

const FIXTURE_VERSION: &str = "0.147.0";
const FIXTURES: [(&str, &str, &str); 6] = [
    (
        "agent-turn-complete.json",
        include_str!("fixtures/codex/0.147.0/agent-turn-complete.json"),
        "agent-turn-complete",
    ),
    (
        "session-start.json",
        include_str!("fixtures/codex/0.147.0/session-start.json"),
        "SessionStart",
    ),
    (
        "user-prompt-submit.json",
        include_str!("fixtures/codex/0.147.0/user-prompt-submit.json"),
        "UserPromptSubmit",
    ),
    (
        "permission-request.json",
        include_str!("fixtures/codex/0.147.0/permission-request.json"),
        "PermissionRequest",
    ),
    (
        "stop.json",
        include_str!("fixtures/codex/0.147.0/stop.json"),
        "Stop",
    ),
    (
        "session-end.json",
        include_str!("fixtures/codex/0.147.0/session-end.json"),
        "SessionEnd",
    ),
];

#[test]
fn codex_golden_fixtures_match_the_versioned_surface_manifest() {
    let manifest: Value = serde_json::from_str(include_str!(
        "fixtures/codex/0.147.0/manifest.json"
    ))
    .unwrap();
    assert_eq!(manifest["codexVersion"], FIXTURE_VERSION);
    assert_eq!(manifest["sourceTag"], "rust-v0.147.0");
    let surfaces = manifest["surfaces"].as_array().unwrap();

    for (file_name, fixture, expected_surface) in FIXTURES {
        let payload: Value = serde_json::from_str(fixture)
            .unwrap_or_else(|error| panic!("{file_name} is not valid JSON: {error}"));
        let actual_surface = payload
            .get("type")
            .or_else(|| payload.get("hook_event_name"))
            .and_then(Value::as_str);
        assert_eq!(actual_surface, Some(expected_surface), "{file_name}");
        assert!(
            surfaces.iter().any(|surface| surface == expected_surface),
            "{file_name} is missing from the fixture manifest"
        );
    }
}

#[test]
fn lifecycle_fixtures_keep_public_identity_and_context_fields() {
    for (file_name, fixture, surface) in FIXTURES {
        let payload: Value = serde_json::from_str(fixture).unwrap();
        if surface == "agent-turn-complete" {
            assert!(payload["thread-id"].is_string(), "{file_name}");
            assert!(payload["turn-id"].is_string(), "{file_name}");
        } else {
            assert!(payload["session_id"].is_string(), "{file_name}");
        }
        assert!(payload["cwd"].is_string(), "{file_name}");
    }
}
