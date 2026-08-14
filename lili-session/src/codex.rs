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
const PLUGIN_SOURCE_PREFIX: &str = "plugin:";

pub const TESTED_CODEX_VERSION: &str = "0.147.0";
pub const DESKTOP_VERSION: &str = env!("CARGO_PKG_VERSION");

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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexPluginSupport {
    Supported,
    Unreviewed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexPluginAvailability {
    Installed,
    Available,
    NotAvailable,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexHookSource {
    None,
    Legacy,
    Plugin,
    Overlap,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexPluginTrustState {
    NotApplicable,
    Unknown,
    TrustedAtLastDelivery,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexPluginIpcCompatibility {
    Supported,
    Unsupported,
    PackageMismatch,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastAcceptedCodexEvent {
    pub event_id: String,
    pub event_type: crate::SessionEventKind,
    pub occurred_at_ms: u64,
    pub surface: CodexIntegrationSurface,
    pub plugin_version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPluginDiagnostics {
    pub codex_support: CodexPluginSupport,
    pub availability: CodexPluginAvailability,
    #[serde(default)]
    pub plugin_id: Option<String>,
    pub installed: Option<bool>,
    pub enabled: Option<bool>,
    pub hook_source: CodexHookSource,
    pub trust_state: CodexPluginTrustState,
    pub plugin_version: Option<String>,
    pub desktop_version: String,
    pub ipc_compatibility: CodexPluginIpcCompatibility,
    pub last_accepted_plugin_event: Option<LastAcceptedCodexEvent>,
    pub remediation: Vec<String>,
}

impl CodexPluginDiagnostics {
    pub fn unavailable(codex_version: Option<&str>, legacy_active: bool) -> Self {
        let codex_support = match bounded_version(codex_version) {
            Some(version) if version == TESTED_CODEX_VERSION => CodexPluginSupport::Supported,
            Some(_) => CodexPluginSupport::Unreviewed,
            None => CodexPluginSupport::Unknown,
        };
        Self {
            codex_support,
            availability: CodexPluginAvailability::Unknown,
            plugin_id: None,
            installed: None,
            enabled: None,
            hook_source: if legacy_active {
                CodexHookSource::Legacy
            } else {
                CodexHookSource::Unknown
            },
            trust_state: CodexPluginTrustState::Unknown,
            plugin_version: None,
            desktop_version: DESKTOP_VERSION.to_owned(),
            ipc_compatibility: CodexPluginIpcCompatibility::Unknown,
            last_accepted_plugin_event: None,
            remediation: vec![
                "Run `codex plugin list --available --json` before changing the current integration."
                    .to_owned(),
            ],
        }
    }

    pub fn discovered(
        codex_version: Option<&str>,
        availability: CodexPluginAvailability,
        installed: bool,
        enabled: bool,
        plugin_version: Option<&str>,
        legacy_active: bool,
    ) -> Self {
        let plugin_version = bounded_version(plugin_version);
        let codex_support = match bounded_version(codex_version) {
            Some(version) if version == TESTED_CODEX_VERSION => CodexPluginSupport::Supported,
            Some(_) => CodexPluginSupport::Unreviewed,
            None => CodexPluginSupport::Unknown,
        };
        let plugin_active = installed && enabled;
        let hook_source = match (legacy_active, plugin_active) {
            (true, true) => CodexHookSource::Overlap,
            (true, false) => CodexHookSource::Legacy,
            (false, true) => CodexHookSource::Plugin,
            (false, false) => CodexHookSource::None,
        };
        let ipc_compatibility = if installed {
            match plugin_version.as_deref() {
                Some(version) if supported_release_version(version) => {
                    CodexPluginIpcCompatibility::Supported
                }
                Some(_) => CodexPluginIpcCompatibility::Unsupported,
                None => CodexPluginIpcCompatibility::Unknown,
            }
        } else {
            CodexPluginIpcCompatibility::Unknown
        };
        let trust_state = if installed && enabled {
            CodexPluginTrustState::Unknown
        } else {
            CodexPluginTrustState::NotApplicable
        };
        let remediation = plugin_remediation(
            codex_support,
            availability,
            installed,
            enabled,
            ipc_compatibility,
        );
        Self {
            codex_support,
            availability,
            plugin_id: None,
            installed: Some(installed),
            enabled: Some(enabled),
            hook_source,
            trust_state,
            plugin_version,
            desktop_version: DESKTOP_VERSION.to_owned(),
            ipc_compatibility,
            last_accepted_plugin_event: None,
            remediation,
        }
    }

    pub fn with_plugin_id(mut self, plugin_id: Option<&str>) -> Self {
        self.plugin_id = plugin_id.and_then(bounded_plugin_id);
        self
    }

    fn record_plugin_event(&mut self, event: LastAcceptedCodexEvent) {
        let observed_version = event.plugin_version.as_deref();
        if let (Some(installed), Some(observed)) =
            (self.plugin_version.as_deref(), observed_version)
            && installed != observed
        {
            self.ipc_compatibility = CodexPluginIpcCompatibility::PackageMismatch;
        } else if observed_version.is_some_and(supported_release_version) {
            self.ipc_compatibility = CodexPluginIpcCompatibility::Supported;
        }
        self.hook_source = match self.hook_source {
            CodexHookSource::Legacy | CodexHookSource::Overlap => CodexHookSource::Overlap,
            _ => CodexHookSource::Plugin,
        };
        self.trust_state = CodexPluginTrustState::TrustedAtLastDelivery;
        self.last_accepted_plugin_event = Some(event);
    }
}

fn bounded_plugin_id(plugin_id: &str) -> Option<String> {
    if plugin_id.is_empty()
        || plugin_id.len() > 128
        || plugin_id
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'-' | b'_' | b'@'))
        || plugin_id.matches('@').count() != 1
    {
        return None;
    }
    Some(plugin_id.to_owned())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexAdapterDiagnostics {
    pub tested_codex_version: String,
    pub codex_version: Option<String>,
    pub discovered_surfaces: Vec<CodexIntegrationSurface>,
    pub missing_lifecycle_coverage: Vec<MissingLifecycleCoverage>,
    pub last_accepted_event: Option<LastAcceptedCodexEvent>,
    pub plugin: CodexPluginDiagnostics,
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
        let plugin = CodexPluginDiagnostics::unavailable(codex_version.as_deref(), false);
        Self {
            tested_codex_version: TESTED_CODEX_VERSION.to_owned(),
            codex_version,
            discovered_surfaces,
            missing_lifecycle_coverage,
            last_accepted_event: None,
            plugin,
            remediation,
        }
    }

    pub fn with_plugin(mut self, plugin: CodexPluginDiagnostics) -> Self {
        self.plugin = plugin;
        self
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
        let accepted = LastAcceptedCodexEvent {
            event_id: event.event_id.as_str().to_owned(),
            event_type: event.event_type,
            occurred_at_ms: event.occurred_at_ms,
            surface,
            plugin_version: plugin_version_from_source(&event.source_discriminator)
                .map(str::to_owned),
        };
        if accepted.plugin_version.is_some() {
            self.plugin.record_plugin_event(accepted.clone());
        }
        self.last_accepted_event = Some(accepted);
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
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
    tool_use_id: Option<String>,
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

pub fn mark_plugin_hook_event(event: &mut NormalizedSessionEvent) -> bool {
    if event.provider.as_str() != "codex" || !event.source_discriminator.starts_with("hook:") {
        return false;
    }
    let prefix = format!("{PLUGIN_SOURCE_PREFIX}{DESKTOP_VERSION}:");
    let remaining = MAX_SOURCE_DISCRIMINATOR_CHARS.saturating_sub(prefix.chars().count());
    let source = event
        .source_discriminator
        .chars()
        .take(remaining)
        .collect::<String>();
    event.source_discriminator = format!("{prefix}{source}");
    true
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
    if hook == PERMISSION_REQUEST_HOOK && input.tool_use_id.is_none() {
        return Err(NormalizationError::MissingField("tool use identity"));
    }
    let event_id = lifecycle_event_id(&input, hook);
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
        event_id,
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

fn lifecycle_event_id(input: &LifecycleInput, hook: &str) -> Option<String> {
    let session_id = input.session_id.as_deref()?;
    let mut digest = Sha256::new();
    update_identity_field(&mut digest, hook.as_bytes());
    update_identity_field(&mut digest, session_id.as_bytes());

    match hook {
        USER_PROMPT_SUBMIT_HOOK | STOP_HOOK => {
            update_identity_field(&mut digest, input.turn_id.as_deref()?.as_bytes());
        }
        PERMISSION_REQUEST_HOOK => {
            update_identity_field(&mut digest, input.turn_id.as_deref()?.as_bytes());
            update_identity_field(&mut digest, input.tool_use_id.as_deref()?.as_bytes());
            update_optional_identity_field(&mut digest, input.tool_name.as_deref());
            let tool_input = serde_json::to_vec(input.tool_input.as_ref()?).ok()?;
            update_identity_field(&mut digest, &tool_input);
        }
        SESSION_START_HOOK | SESSION_END_HOOK => {}
        _ => return None,
    }

    let mut event_id = String::from("codex-hook-");
    for byte in digest.finalize() {
        write!(&mut event_id, "{byte:02x}").expect("writing to a string cannot fail");
    }
    Some(event_id)
}

fn update_optional_identity_field(digest: &mut Sha256, field: Option<&str>) {
    match field {
        Some(field) => {
            digest.update([1]);
            update_identity_field(digest, field.as_bytes());
        }
        None => digest.update([0]),
    }
}

fn update_identity_field(digest: &mut Sha256, field: &[u8]) {
    digest.update((field.len() as u64).to_be_bytes());
    digest.update(field);
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
    let source = plugin_version_from_source(source)
        .and_then(|_| source.splitn(3, ':').nth(2))
        .unwrap_or(source);
    match source.split(':').nth(1) {
        Some(SESSION_START_HOOK) => Some(CodexIntegrationSurface::SessionStart),
        Some(USER_PROMPT_SUBMIT_HOOK) => Some(CodexIntegrationSurface::UserPromptSubmit),
        Some(PERMISSION_REQUEST_HOOK) => Some(CodexIntegrationSurface::PermissionRequest),
        Some(STOP_HOOK) => Some(CodexIntegrationSurface::Stop),
        Some(SESSION_END_HOOK) => Some(CodexIntegrationSurface::SessionEnd),
        _ => None,
    }
}

fn plugin_version_from_source(source: &str) -> Option<&str> {
    let remainder = source.strip_prefix(PLUGIN_SOURCE_PREFIX)?;
    let (version, nested_source) = remainder.split_once(':')?;
    if !release_version(version) || !nested_source.starts_with("hook:") {
        return None;
    }
    Some(version)
}

fn supported_release_version(version: &str) -> bool {
    release_version(version) && version.starts_with("0.1.")
}

fn release_version(version: &str) -> bool {
    let mut components = version.split('.');
    matches!(
        (
            components.next().and_then(parse_release_component),
            components.next().and_then(parse_release_component),
            components.next().and_then(parse_release_component),
            components.next(),
        ),
        (Some(_), Some(_), Some(_), None)
    )
}

fn parse_release_component(value: &str) -> Option<u64> {
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) {
        return None;
    }
    value.parse().ok()
}

fn plugin_remediation(
    codex_support: CodexPluginSupport,
    availability: CodexPluginAvailability,
    installed: bool,
    enabled: bool,
    compatibility: CodexPluginIpcCompatibility,
) -> Vec<String> {
    let mut guidance = Vec::new();
    if codex_support != CodexPluginSupport::Supported {
        guidance
            .push("Keep the current integration until this Codex version is reviewed.".to_owned());
    }
    if availability == CodexPluginAvailability::Available && !installed {
        guidance
            .push("Install the Lili plugin with the supported Codex plugin command.".to_owned());
    } else if availability == CodexPluginAvailability::NotAvailable {
        guidance.push(
            "Keep the current integration because the Lili plugin is not available.".to_owned(),
        );
    } else if installed && !enabled {
        guidance.push(
            "Enable the installed Lili plugin, then review its hook trust prompt.".to_owned(),
        );
    } else if installed && enabled {
        guidance.push("Review the exact hook definitions and wait for a real plugin event before legacy cleanup.".to_owned());
    }
    if compatibility == CodexPluginIpcCompatibility::Unsupported {
        guidance.push(
            "Use matching supported Lili plugin and desktop versions before migration.".to_owned(),
        );
    } else if compatibility == CodexPluginIpcCompatibility::PackageMismatch {
        guidance
            .push("Disable the plugin and reinstall the matching unmodified package.".to_owned());
    }
    guidance
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
    fn lifecycle_identity_is_stable_across_delivery_time() {
        for (payload, _, _) in LIFECYCLE_FIXTURES {
            let first = normalize_lifecycle_json(payload, 42).unwrap();
            let second = normalize_lifecycle_json(payload, 99).unwrap();
            assert_eq!(first.event_id, second.event_id);
            assert_eq!(second.occurred_at_ms, 99);
        }
    }

    #[test]
    fn permission_identity_distinguishes_requests_without_retaining_arguments() {
        let first = normalize_lifecycle_json(LIFECYCLE_FIXTURES[2].0, 42).unwrap();
        let mut changed: serde_json::Value =
            serde_json::from_slice(LIFECYCLE_FIXTURES[2].0).unwrap();
        changed["tool_use_id"] = serde_json::json!("toolu_02");
        let second = normalize_lifecycle_json(&serde_json::to_vec(&changed).unwrap(), 42).unwrap();

        assert_ne!(first.event_id, second.event_id);
        let normalized = serde_json::to_string(&second).unwrap();
        assert!(!normalized.contains("cargo test"));
        assert!(!normalized.contains("tool_input"));
        assert!(!normalized.contains("toolu_02"));

        changed.as_object_mut().unwrap().remove("tool_use_id");
        assert_eq!(
            normalize_lifecycle_json(&serde_json::to_vec(&changed).unwrap(), 42),
            Err(NormalizationError::MissingField("tool use identity"))
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

    #[test]
    fn plugin_attribution_preserves_legacy_event_identity() {
        let legacy = normalize_lifecycle_json(LIFECYCLE_FIXTURES[3].0, 42).unwrap();
        let mut plugin = legacy.clone();
        assert!(mark_plugin_hook_event(&mut plugin));
        assert_eq!(plugin.event_id, legacy.event_id);
        assert_eq!(
            plugin.source_discriminator,
            format!("plugin:{DESKTOP_VERSION}:hook:Stop")
        );
        assert!(plugin.validate().is_ok());
    }

    #[test]
    fn plugin_diagnostics_require_observed_delivery_for_trust() {
        let plugin = CodexPluginDiagnostics::discovered(
            Some(TESTED_CODEX_VERSION),
            CodexPluginAvailability::Installed,
            true,
            true,
            Some(DESKTOP_VERSION),
            true,
        );
        let mut diagnostics = CodexAdapterDiagnostics::with_discovery(
            Some(TESTED_CODEX_VERSION),
            [CodexIntegrationSurface::Stop],
        )
        .with_plugin(plugin);
        assert_eq!(
            diagnostics.plugin.trust_state,
            CodexPluginTrustState::Unknown
        );
        assert_eq!(diagnostics.plugin.hook_source, CodexHookSource::Overlap);

        let mut event = normalize_lifecycle_json(LIFECYCLE_FIXTURES[3].0, 42).unwrap();
        assert!(mark_plugin_hook_event(&mut event));
        diagnostics.record_accepted_event(&event);

        assert_eq!(
            diagnostics.plugin.trust_state,
            CodexPluginTrustState::TrustedAtLastDelivery
        );
        assert_eq!(
            diagnostics
                .plugin
                .last_accepted_plugin_event
                .as_ref()
                .and_then(|event| event.plugin_version.as_deref()),
            Some(DESKTOP_VERSION)
        );
    }
}
