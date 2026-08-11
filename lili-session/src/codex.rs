use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    MAX_PROVIDER_PAYLOAD_BYTES, NormalizationError, NormalizedSessionEvent,
    ProviderCapabilitiesInputV1, ProviderInputV1, ProviderProjectInputV1, SESSION_SCHEMA_VERSION,
    normalize_json, normalize_provider_input,
};

const NOTIFY_EVENT_TYPE: &str = "agent-turn-complete";
const MAX_SOURCE_DISCRIMINATOR_CHARS: usize = 128;

const SESSION_START_HOOK: &str = "SessionStart";
const USER_PROMPT_SUBMIT_HOOK: &str = "UserPromptSubmit";
const PERMISSION_REQUEST_HOOK: &str = "PermissionRequest";
const STOP_HOOK: &str = "Stop";
const SESSION_END_HOOK: &str = "SessionEnd";

pub const TESTED_CODEX_VERSION: &str = "0.147.0";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexIntegrationSurface {
    Notify,
    SessionStart,
    UserPromptSubmit,
    PermissionRequest,
    Stop,
    SessionEnd,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingLifecycleCoverage {
    SessionStart,
    Active,
    Attention,
    Completion,
    SessionEnd,
    Failure,
    AttentionResolution,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastAcceptedCodexEvent {
    pub event_id: String,
    pub event_type: crate::SessionEventKind,
    pub occurred_at_ms: u64,
    pub surface: CodexIntegrationSurface,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAdapterDiagnostics {
    pub tested_codex_version: String,
    pub codex_version: Option<String>,
    pub discovered_surfaces: Vec<CodexIntegrationSurface>,
    pub missing_lifecycle_coverage: Vec<MissingLifecycleCoverage>,
    pub last_accepted_event: Option<LastAcceptedCodexEvent>,
    pub remediation: Vec<String>,
}

impl CodexAdapterDiagnostics {
    pub fn with_discovery(
        codex_version: Option<&str>,
        surfaces: impl IntoIterator<Item = CodexIntegrationSurface>,
    ) -> Self {
        let mut discovered_surfaces = surfaces.into_iter().collect::<Vec<_>>();
        discovered_surfaces.sort_unstable();
        discovered_surfaces.dedup();
        let codex_version = bounded_version(codex_version);
        let missing_lifecycle_coverage = missing_coverage(&discovered_surfaces);
        let remediation = remediation(codex_version.as_deref(), &missing_lifecycle_coverage);
        Self {
            tested_codex_version: TESTED_CODEX_VERSION.to_owned(),
            codex_version,
            discovered_surfaces,
            missing_lifecycle_coverage,
            last_accepted_event: None,
            remediation,
        }
    }

    pub fn record_accepted_event(&mut self, event: &NormalizedSessionEvent) {
        if event.provider.as_str() != "codex" {
            return;
        }
        let Some(surface) = surface_from_source(&event.source_discriminator) else {
            return;
        };
        if let Err(index) = self.discovered_surfaces.binary_search(&surface) {
            self.discovered_surfaces.insert(index, surface);
        }
        self.missing_lifecycle_coverage = missing_coverage(&self.discovered_surfaces);
        self.remediation = remediation(
            self.codex_version.as_deref(),
            &self.missing_lifecycle_coverage,
        );
        self.last_accepted_event = Some(LastAcceptedCodexEvent {
            event_id: event.event_id.as_str().to_owned(),
            event_type: event.event_type,
            occurred_at_ms: event.occurred_at_ms,
            surface,
        });
    }
}

impl Default for CodexAdapterDiagnostics {
    fn default() -> Self {
        Self::with_discovery(None, [])
    }
}

#[derive(Debug, Deserialize)]
struct NotifyInput {
    #[serde(rename = "type")]
    event_type: Option<String>,
    #[serde(rename = "thread-id")]
    thread_id: Option<String>,
    #[serde(rename = "turn-id")]
    turn_id: Option<String>,
    cwd: Option<String>,
    client: Option<String>,
    #[serde(rename = "last-assistant-message")]
    last_assistant_message: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LifecycleInput {
    session_id: Option<String>,
    turn_id: Option<String>,
    cwd: Option<String>,
    hook_event_name: Option<String>,
    source: Option<String>,
    last_assistant_message: Option<String>,
}

pub fn normalize_hook_json(
    payload: &[u8],
    occurred_at_ms: u64,
) -> Result<NormalizedSessionEvent, NormalizationError> {
    if payload.len() > MAX_PROVIDER_PAYLOAD_BYTES {
        return Err(NormalizationError::PayloadTooLarge);
    }
    let value: serde_json::Value =
        serde_json::from_slice(payload).map_err(|_| NormalizationError::MalformedJson)?;
    if value.get("type").and_then(serde_json::Value::as_str) == Some(NOTIFY_EVENT_TYPE) {
        return normalize_notify_json(payload, occurred_at_ms);
    }
    if value.get("hook_event_name").is_some() {
        return normalize_lifecycle_json(payload, occurred_at_ms);
    }
    normalize_json(payload)
}

pub fn normalize_notify_json(
    payload: &[u8],
    occurred_at_ms: u64,
) -> Result<NormalizedSessionEvent, NormalizationError> {
    if payload.len() > MAX_PROVIDER_PAYLOAD_BYTES {
        return Err(NormalizationError::PayloadTooLarge);
    }
    let input: NotifyInput =
        serde_json::from_slice(payload).map_err(|_| NormalizationError::MalformedJson)?;
    match input.event_type.as_deref() {
        Some(NOTIFY_EVENT_TYPE) => {}
        Some(_) => return Err(NormalizationError::UnsupportedEventType),
        None => return Err(NormalizationError::MissingField("event type")),
    }

    let event_id = notify_event_id(input.thread_id.as_deref(), input.turn_id.as_deref());
    let source_discriminator = notify_source(input.client.as_deref())?;
    normalize_provider_input(ProviderInputV1 {
        version: SESSION_SCHEMA_VERSION,
        provider: Some("codex".to_owned()),
        event_type: Some(NOTIFY_EVENT_TYPE.to_owned()),
        event_id,
        session_id: input.thread_id,
        turn_id: input.turn_id,
        occurred_at_ms: Some(occurred_at_ms),
        project: input
            .cwd
            .map(|label| ProviderProjectInputV1 { label: Some(label) }),
        summary: input.last_assistant_message,
        capabilities: ProviderCapabilitiesInputV1 {
            reports_completion: true,
            ..ProviderCapabilitiesInputV1::default()
        },
        source_discriminator: Some(source_discriminator),
    })
}

pub fn normalize_lifecycle_json(
    payload: &[u8],
    occurred_at_ms: u64,
) -> Result<NormalizedSessionEvent, NormalizationError> {
    if payload.len() > MAX_PROVIDER_PAYLOAD_BYTES {
        return Err(NormalizationError::PayloadTooLarge);
    }
    let input: LifecycleInput =
        serde_json::from_slice(payload).map_err(|_| NormalizationError::MalformedJson)?;
    let hook = input
        .hook_event_name
        .as_deref()
        .ok_or(NormalizationError::MissingField("hook event name"))?;
    let (event_type, turn_id, summary) = match hook {
        SESSION_START_HOOK => ("session_started", None, None),
        USER_PROMPT_SUBMIT_HOOK => ("turn_started", input.turn_id, None),
        PERMISSION_REQUEST_HOOK => ("attention_required", input.turn_id, None),
        STOP_HOOK => (
            "turn_completed",
            input.turn_id,
            input.last_assistant_message,
        ),
        SESSION_END_HOOK => ("session_ended", None, None),
        _ => return Err(NormalizationError::UnsupportedEventType),
    };
    let source_discriminator = lifecycle_source(hook, input.source.as_deref())?;
    normalize_provider_input(ProviderInputV1 {
        version: SESSION_SCHEMA_VERSION,
        provider: Some("codex".to_owned()),
        event_type: Some(event_type.to_owned()),
        event_id: None,
        session_id: input.session_id,
        turn_id,
        occurred_at_ms: Some(occurred_at_ms),
        project: input
            .cwd
            .map(|label| ProviderProjectInputV1 { label: Some(label) }),
        summary,
        capabilities: lifecycle_capabilities(),
        source_discriminator: Some(source_discriminator),
    })
}

fn lifecycle_capabilities() -> ProviderCapabilitiesInputV1 {
    ProviderCapabilitiesInputV1 {
        reports_active: true,
        reports_attention: true,
        reports_completion: true,
        reports_failure: false,
        reports_resolution: false,
        reports_session_end: true,
    }
}

fn notify_event_id(thread_id: Option<&str>, turn_id: Option<&str>) -> Option<String> {
    let (Some(thread_id), Some(turn_id)) = (thread_id, turn_id) else {
        return None;
    };
    let mut digest = Sha256::new();
    for field in [NOTIFY_EVENT_TYPE, thread_id, turn_id] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field.as_bytes());
    }
    let mut event_id = String::from("codex-notify-");
    for byte in digest.finalize() {
        write!(&mut event_id, "{byte:02x}").expect("writing to a string cannot fail");
    }
    Some(event_id)
}

fn notify_source(client: Option<&str>) -> Result<String, NormalizationError> {
    let Some(client) = client else {
        return Ok("notify".to_owned());
    };
    let client = client.trim();
    if client.is_empty() {
        return Ok("notify".to_owned());
    }
    if client.chars().any(char::is_control) {
        return Err(NormalizationError::InvalidField("client identity"));
    }
    let prefix = "notify:";
    let limit = MAX_SOURCE_DISCRIMINATOR_CHARS - prefix.chars().count();
    let client = client.chars().take(limit).collect::<String>();
    Ok(format!("{prefix}{client}"))
}

fn lifecycle_source(
    hook: &str,
    session_start_source: Option<&str>,
) -> Result<String, NormalizationError> {
    let mut source = format!("hook:{hook}");
    if hook == SESSION_START_HOOK
        && let Some(start_source) = session_start_source
    {
        let start_source = start_source.trim();
        if start_source.is_empty() || start_source.chars().any(char::is_control) {
            return Err(NormalizationError::InvalidField("session start source"));
        }
        source.push(':');
        source.extend(
            start_source
                .chars()
                .take(MAX_SOURCE_DISCRIMINATOR_CHARS.saturating_sub(source.chars().count())),
        );
    }
    Ok(source)
}

fn bounded_version(version: Option<&str>) -> Option<String> {
    let version = version?.trim();
    if version.is_empty() || version.len() > 64 || version.chars().any(char::is_control) {
        return None;
    }
    Some(version.to_owned())
}

fn surface_from_source(source: &str) -> Option<CodexIntegrationSurface> {
    if source == "notify" || source.starts_with("notify:") {
        return Some(CodexIntegrationSurface::Notify);
    }
    match source.split(':').nth(1) {
        Some(SESSION_START_HOOK) => Some(CodexIntegrationSurface::SessionStart),
        Some(USER_PROMPT_SUBMIT_HOOK) => Some(CodexIntegrationSurface::UserPromptSubmit),
        Some(PERMISSION_REQUEST_HOOK) => Some(CodexIntegrationSurface::PermissionRequest),
        Some(STOP_HOOK) => Some(CodexIntegrationSurface::Stop),
        Some(SESSION_END_HOOK) => Some(CodexIntegrationSurface::SessionEnd),
        _ => None,
    }
}

fn missing_coverage(surfaces: &[CodexIntegrationSurface]) -> Vec<MissingLifecycleCoverage> {
    let has = |surface| surfaces.binary_search(&surface).is_ok();
    let mut missing = Vec::new();
    if !has(CodexIntegrationSurface::SessionStart) {
        missing.push(MissingLifecycleCoverage::SessionStart);
    }
    if !has(CodexIntegrationSurface::UserPromptSubmit) {
        missing.push(MissingLifecycleCoverage::Active);
    }
    if !has(CodexIntegrationSurface::PermissionRequest) {
        missing.push(MissingLifecycleCoverage::Attention);
    }
    if !has(CodexIntegrationSurface::Notify) && !has(CodexIntegrationSurface::Stop) {
        missing.push(MissingLifecycleCoverage::Completion);
    }
    if !has(CodexIntegrationSurface::SessionEnd) {
        missing.push(MissingLifecycleCoverage::SessionEnd);
    }
    missing.push(MissingLifecycleCoverage::Failure);
    missing.push(MissingLifecycleCoverage::AttentionResolution);
    missing
}

fn remediation(codex_version: Option<&str>, missing: &[MissingLifecycleCoverage]) -> Vec<String> {
    let mut guidance = Vec::new();
    if codex_version.is_none() {
        guidance.push("Run lili integrate inspect before changing Codex configuration.".to_owned());
    } else if codex_version != Some(TESTED_CODEX_VERSION) {
        guidance
            .push("Review version-keyed fixture compatibility before enabling changes.".to_owned());
    }
    if missing.iter().any(|coverage| {
        !matches!(
            coverage,
            MissingLifecycleCoverage::Failure | MissingLifecycleCoverage::AttentionResolution
        )
    }) {
        guidance.push(
            "Preview integration changes and enable only documented hook surfaces.".to_owned(),
        );
    }
    if missing.contains(&MissingLifecycleCoverage::Failure)
        || missing.contains(&MissingLifecycleCoverage::AttentionResolution)
    {
        guidance.push(
            "Do not infer unsupported lifecycle distinctions from unrelated hooks.".to_owned(),
        );
    }
    guidance
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SessionEventKind;

    const NOTIFY_FIXTURE: &[u8] =
        include_bytes!("../tests/fixtures/codex/0.147.0/agent-turn-complete.json");
    const LIFECYCLE_FIXTURES: [(&[u8], SessionEventKind, bool); 5] = [
        (
            include_bytes!("../tests/fixtures/codex/0.147.0/session-start.json"),
            SessionEventKind::SessionStarted,
            false,
        ),
        (
            include_bytes!("../tests/fixtures/codex/0.147.0/user-prompt-submit.json"),
            SessionEventKind::TurnStarted,
            true,
        ),
        (
            include_bytes!("../tests/fixtures/codex/0.147.0/permission-request.json"),
            SessionEventKind::AttentionRequired,
            true,
        ),
        (
            include_bytes!("../tests/fixtures/codex/0.147.0/stop.json"),
            SessionEventKind::TurnCompleted,
            true,
        ),
        (
            include_bytes!("../tests/fixtures/codex/0.147.0/session-end.json"),
            SessionEventKind::SessionEnded,
            false,
        ),
    ];

    #[test]
    fn notify_fixture_normalizes_terminal_turn_metadata() {
        let event = normalize_notify_json(NOTIFY_FIXTURE, 42).unwrap();
        assert_eq!(event.provider.as_str(), "codex");
        assert_eq!(event.event_type, SessionEventKind::TurnCompleted);
        assert_eq!(
            event.session_id.as_str(),
            "01912f9d-3109-722d-a391-8e7b42ab1d31"
        );
        assert_eq!(event.turn_id.as_ref().unwrap().as_str(), "turn-018f");
        assert_eq!(event.occurred_at_ms, 42);
        assert_eq!(event.project.as_ref().unwrap().label(), "lili-fixture");
        assert_eq!(
            event.summary.as_ref().unwrap().text(),
            "Fixture verification completed successfully."
        );
        assert_eq!(event.source_discriminator, "notify:codex-tui");
        assert!(event.capabilities.reports_completion);
    }

    #[test]
    fn notify_identity_is_stable_across_delivery_time_and_ignores_prompts() {
        let first = normalize_notify_json(NOTIFY_FIXTURE, 42).unwrap();
        let mut changed: serde_json::Value = serde_json::from_slice(NOTIFY_FIXTURE).unwrap();
        changed["input-messages"] = serde_json::json!(["different private prompt"]);
        let second = normalize_notify_json(&serde_json::to_vec(&changed).unwrap(), 99).unwrap();
        assert_eq!(first.event_id, second.event_id);
        assert_eq!(second.occurred_at_ms, 99);
    }

    #[test]
    fn notify_summary_uses_existing_bounds_and_redaction() {
        let mut payload: serde_json::Value = serde_json::from_slice(NOTIFY_FIXTURE).unwrap();
        payload["last-assistant-message"] =
            serde_json::Value::String(format!("token=secret\n{}", "x".repeat(400)));
        let event = normalize_notify_json(&serde_json::to_vec(&payload).unwrap(), 42).unwrap();
        let summary = event.summary.unwrap();
        assert!(summary.was_redacted());
        assert!(!summary.text().contains("secret"));
    }

    #[test]
    fn hook_normalizer_preserves_provider_neutral_test_inputs() {
        let payload = br#"{"version":1,"provider":"codex","type":"turn_completed","sessionId":"session-1","turnId":"turn-1","occurredAtMs":42}"#;
        assert!(normalize_hook_json(payload, 100).is_ok());
    }

    #[test]
    fn lifecycle_fixtures_map_only_documented_distinctions() {
        for (payload, expected_kind, expects_turn) in LIFECYCLE_FIXTURES {
            let event = normalize_lifecycle_json(payload, 42).unwrap();
            assert_eq!(event.event_type, expected_kind);
            assert_eq!(event.turn_id.is_some(), expects_turn);
            assert!(event.capabilities.reports_active);
            assert!(event.capabilities.reports_attention);
            assert!(event.capabilities.reports_completion);
            assert!(event.capabilities.reports_session_end);
            assert!(!event.capabilities.reports_failure);
            assert!(!event.capabilities.reports_resolution);
        }
    }

    #[test]
    fn lifecycle_adapter_does_not_retain_prompt_or_permission_details() {
        for payload in [LIFECYCLE_FIXTURES[1].0, LIFECYCLE_FIXTURES[2].0] {
            let event = normalize_lifecycle_json(payload, 42).unwrap();
            assert!(event.summary.is_none());
        }
        let stop = normalize_lifecycle_json(LIFECYCLE_FIXTURES[3].0, 42).unwrap();
        assert_eq!(
            stop.summary.unwrap().text(),
            "Fixture verification completed successfully."
        );
    }

    #[test]
    fn lifecycle_adapter_rejects_unknown_hooks_without_inference() {
        let payload = br#"{"hook_event_name":"PostToolUse","session_id":"session-1","turn_id":"turn-1","cwd":"/tmp/project"}"#;
        assert_eq!(
            normalize_lifecycle_json(payload, 42),
            Err(NormalizationError::UnsupportedEventType)
        );
    }

    #[test]
    fn compatibility_diagnostics_are_honest_about_missing_coverage() {
        let mut diagnostics = CodexAdapterDiagnostics::with_discovery(
            Some(TESTED_CODEX_VERSION),
            [
                CodexIntegrationSurface::Notify,
                CodexIntegrationSurface::SessionStart,
                CodexIntegrationSurface::UserPromptSubmit,
                CodexIntegrationSurface::PermissionRequest,
                CodexIntegrationSurface::Stop,
                CodexIntegrationSurface::SessionEnd,
            ],
        );
        assert_eq!(diagnostics.codex_version.as_deref(), Some("0.147.0"));
        assert_eq!(
            diagnostics.missing_lifecycle_coverage,
            [
                MissingLifecycleCoverage::Failure,
                MissingLifecycleCoverage::AttentionResolution
            ]
        );

        let event = normalize_notify_json(NOTIFY_FIXTURE, 42).unwrap();
        diagnostics.record_accepted_event(&event);
        let last = diagnostics.last_accepted_event.unwrap();
        assert_eq!(last.surface, CodexIntegrationSurface::Notify);
        assert_eq!(last.event_type, SessionEventKind::TurnCompleted);
    }

    #[test]
    fn unknown_or_unverified_versions_get_safe_remediation() {
        let unknown = CodexAdapterDiagnostics::default();
        assert!(unknown.codex_version.is_none());
        assert!(
            unknown
                .remediation
                .iter()
                .any(|guidance| guidance.contains("integrate inspect"))
        );
        let newer = CodexAdapterDiagnostics::with_discovery(Some("0.148.0"), []);
        assert!(
            newer
                .remediation
                .iter()
                .any(|guidance| guidance.contains("version-keyed fixture"))
        );
    }
}
