use std::{fmt, fmt::Write as _};

use sha2::{Digest, Sha256};

use crate::{
    DisplayProjectContext, DisplaySummary, EventId, NormalizedSessionEvent, ProviderId,
    ProviderInputV1, ProviderProjectInputV1, SESSION_SCHEMA_VERSION, SessionEventKind, SessionId,
    SourceCapabilities, TurnId,
    types::{MAX_PROJECT_LABEL_CHARS, MAX_SUMMARY_CHARS},
};

pub const MAX_PROVIDER_PAYLOAD_BYTES: usize = 64 * 1024;
const MAX_SOURCE_DISCRIMINATOR_CHARS: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NormalizationError {
    PayloadTooLarge,
    MalformedJson,
    UnsupportedVersion(u16),
    MissingField(&'static str),
    InvalidField(&'static str),
    UnsupportedEventType,
}

impl fmt::Display for NormalizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PayloadTooLarge => write!(formatter, "provider payload exceeds 64 KiB"),
            Self::MalformedJson => write!(formatter, "provider payload is malformed JSON"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "provider payload version {version} is unsupported"
                )
            }
            Self::MissingField(field) => write!(formatter, "provider payload is missing {field}"),
            Self::InvalidField(field) => write!(formatter, "provider payload has invalid {field}"),
            Self::UnsupportedEventType => write!(formatter, "provider event type is unsupported"),
        }
    }
}

impl std::error::Error for NormalizationError {}

pub fn normalize_json(payload: &[u8]) -> Result<NormalizedSessionEvent, NormalizationError> {
    if payload.len() > MAX_PROVIDER_PAYLOAD_BYTES {
        return Err(NormalizationError::PayloadTooLarge);
    }
    let input = serde_json::from_slice(payload).map_err(|_| NormalizationError::MalformedJson)?;
    normalize_provider_input(input)
}

pub fn normalize_provider_input(
    input: ProviderInputV1,
) -> Result<NormalizedSessionEvent, NormalizationError> {
    if input.version != SESSION_SCHEMA_VERSION {
        return Err(NormalizationError::UnsupportedVersion(input.version));
    }

    let provider = parse_provider(required(input.provider, "provider")?)?;
    let event_type = parse_event_type(required(input.event_type, "event type")?)?;
    let session_id = parse_session(required(input.session_id, "session identity")?)?;
    let turn_id = input.turn_id.map(parse_turn).transpose()?;
    if is_turn_scoped(event_type) && turn_id.is_none() {
        return Err(NormalizationError::MissingField("turn identity"));
    }
    let occurred_at_ms = input
        .occurred_at_ms
        .ok_or(NormalizationError::MissingField("occurrence time"))?;
    let project = input.project.and_then(normalize_project);
    let summary = input.summary.and_then(normalize_summary);
    let source_discriminator = normalize_source_discriminator(input.source_discriminator)?;
    let capabilities = SourceCapabilities::from(input.capabilities);
    let event_id = match input.event_id {
        Some(event_id) => EventId::parse(event_id.trim().to_owned())
            .map_err(|_| NormalizationError::InvalidField("event identity"))?,
        None => fallback_event_id(
            &provider,
            event_type,
            &session_id,
            turn_id.as_ref(),
            occurred_at_ms,
            &source_discriminator,
        ),
    };

    Ok(NormalizedSessionEvent {
        version: SESSION_SCHEMA_VERSION,
        event_id,
        provider,
        event_type,
        session_id,
        turn_id,
        occurred_at_ms,
        project,
        summary,
        capabilities,
        source_discriminator,
    })
}

fn required(value: Option<String>, field: &'static str) -> Result<String, NormalizationError> {
    value
        .filter(|value| !value.trim().is_empty())
        .ok_or(NormalizationError::MissingField(field))
}

fn parse_provider(value: String) -> Result<ProviderId, NormalizationError> {
    ProviderId::parse(value.trim().to_ascii_lowercase())
        .map_err(|_| NormalizationError::InvalidField("provider identity"))
}

fn parse_session(value: String) -> Result<SessionId, NormalizationError> {
    SessionId::parse(value.trim().to_owned())
        .map_err(|_| NormalizationError::InvalidField("session identity"))
}

fn parse_turn(value: String) -> Result<TurnId, NormalizationError> {
    TurnId::parse(value.trim().to_owned())
        .map_err(|_| NormalizationError::InvalidField("turn identity"))
}

fn parse_event_type(value: String) -> Result<SessionEventKind, NormalizationError> {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "session_started" | "session_start" => Ok(SessionEventKind::SessionStarted),
        "turn_started" | "prompt_started" | "active" => Ok(SessionEventKind::TurnStarted),
        "attention_required" | "permission_requested" => Ok(SessionEventKind::AttentionRequired),
        "attention_resolved" => Ok(SessionEventKind::AttentionResolved),
        "turn_completed" | "agent_turn_complete" => Ok(SessionEventKind::TurnCompleted),
        "turn_failed" => Ok(SessionEventKind::TurnFailed),
        "session_ended" | "session_end" => Ok(SessionEventKind::SessionEnded),
        _ => Err(NormalizationError::UnsupportedEventType),
    }
}

fn is_turn_scoped(event_type: SessionEventKind) -> bool {
    !matches!(
        event_type,
        SessionEventKind::SessionStarted | SessionEventKind::SessionEnded
    )
}

fn normalize_project(input: ProviderProjectInputV1) -> Option<DisplayProjectContext> {
    let label = input.label?;
    let basename = label
        .trim()
        .rsplit(['/', '\\'])
        .find(|component| !component.is_empty())?;
    let label = collapse_whitespace(basename);
    let (label, _) = truncate_chars(&label, MAX_PROJECT_LABEL_CHARS);
    DisplayProjectContext::parse(label).ok()
}

fn normalize_summary(summary: String) -> Option<DisplaySummary> {
    let (summary, redacted) = redact_summary(&summary);
    let summary = collapse_whitespace(&summary);
    if summary.is_empty() {
        return None;
    }
    let (summary, truncated) = truncate_chars(&summary, MAX_SUMMARY_CHARS);
    DisplaySummary::parse(summary, truncated, redacted).ok()
}

fn normalize_source_discriminator(source: Option<String>) -> Result<String, NormalizationError> {
    let source = source.unwrap_or_else(|| "default".to_owned());
    let source = collapse_whitespace(&source);
    if source.is_empty()
        || source.chars().count() > MAX_SOURCE_DISCRIMINATOR_CHARS
        || source.chars().any(char::is_control)
    {
        return Err(NormalizationError::InvalidField("source discriminator"));
    }
    Ok(source)
}

fn redact_summary(summary: &str) -> (String, bool) {
    let mut redacted = false;
    let lines = summary.lines().map(|line| {
        let normalized = line.trim().to_ascii_lowercase();
        let sensitive = [
            "authorization:",
            "api_key",
            "api-key",
            "token=",
            "password=",
            "command:",
            "approval arguments",
            "prompt:",
            "user:",
        ]
        .iter()
        .any(|marker| normalized.contains(marker));
        if sensitive {
            redacted = true;
            "[redacted]"
        } else {
            line
        }
    });
    (lines.collect::<Vec<_>>().join(" "), redacted)
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(value: &str, limit: usize) -> (String, bool) {
    if value.chars().count() <= limit {
        return (value.to_owned(), false);
    }
    let mut truncated = value
        .chars()
        .take(limit.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    (truncated, true)
}

fn fallback_event_id(
    provider: &ProviderId,
    event_type: SessionEventKind,
    session_id: &SessionId,
    turn_id: Option<&TurnId>,
    occurred_at_ms: u64,
    source_discriminator: &str,
) -> EventId {
    let occurred_at = occurred_at_ms.to_string();
    let mut digest = Sha256::new();
    for field in [
        provider.as_str(),
        event_type_name(event_type),
        session_id.as_str(),
        turn_id.map_or("", TurnId::as_str),
        occurred_at.as_str(),
        source_discriminator,
    ] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field.as_bytes());
    }
    let mut identity = String::from("fallback-");
    for byte in digest.finalize() {
        write!(&mut identity, "{byte:02x}").expect("writing to a string cannot fail");
    }
    EventId::parse(identity).expect("fallback event identity is bounded")
}

fn event_type_name(event_type: SessionEventKind) -> &'static str {
    match event_type {
        SessionEventKind::SessionStarted => "session_started",
        SessionEventKind::TurnStarted => "turn_started",
        SessionEventKind::AttentionRequired => "attention_required",
        SessionEventKind::AttentionResolved => "attention_resolved",
        SessionEventKind::TurnCompleted => "turn_completed",
        SessionEventKind::TurnFailed => "turn_failed",
        SessionEventKind::SessionEnded => "session_ended",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(extra: &str) -> Vec<u8> {
        format!(
            r#"{{"version":1,"provider":"Codex","type":"agent-turn-complete","sessionId":"session-1","turnId":"turn-1","occurredAtMs":42,"project":{{"label":"/private/work/project"}},"summary":"done"{extra}}}"#
        )
        .into_bytes()
    }

    #[test]
    fn additive_fields_are_ignored_during_normalization() {
        let event = normalize_json(&payload(r#", "futureField":{"nested":true}"#)).unwrap();
        assert_eq!(event.provider.as_str(), "codex");
        assert_eq!(event.event_type, SessionEventKind::TurnCompleted);
        assert_eq!(event.project.unwrap().label(), "project");
    }

    #[test]
    fn missing_required_field_has_bounded_diagnostic() {
        let secret = "do-not-log-this";
        let error = normalize_json(
            format!(
                r#"{{"version":1,"provider":"codex","type":"turn_started","summary":"{secret}"}}"#
            )
            .as_bytes(),
        )
        .unwrap_err();
        assert_eq!(error, NormalizationError::MissingField("session identity"));
        assert!(!error.to_string().contains(secret));
    }

    #[test]
    fn summaries_are_redacted_and_unicode_truncated() {
        let summary = format!(
            "command: dangerous\n{}",
            "猫".repeat(MAX_SUMMARY_CHARS + 20)
        );
        let payload = format!(
            r#"{{"version":1,"provider":"codex","type":"turn_completed","sessionId":"session-1","turnId":"turn-1","occurredAtMs":42,"summary":{}}}"#,
            serde_json::to_string(&summary).unwrap()
        );
        let summary = normalize_json(payload.as_bytes()).unwrap().summary.unwrap();
        assert!(summary.was_redacted());
        assert!(summary.was_truncated());
        assert_eq!(summary.text().chars().count(), MAX_SUMMARY_CHARS);
        assert!(!summary.text().contains("dangerous"));
    }

    #[test]
    fn fallback_event_identity_is_stable_and_field_delimited() {
        let first = normalize_json(&payload("")).unwrap();
        let second = normalize_json(&payload("")).unwrap();
        let mut changed_input: ProviderInputV1 = serde_json::from_slice(&payload("")).unwrap();
        changed_input.source_discriminator = Some("other".to_owned());
        let changed = normalize_provider_input(changed_input).unwrap();
        assert_eq!(first.event_id, second.event_id);
        assert_ne!(first.event_id, changed.event_id);
        assert!(first.event_id.as_str().starts_with("fallback-"));
    }

    #[test]
    fn payload_limit_is_checked_before_parsing() {
        let payload = vec![b'x'; MAX_PROVIDER_PAYLOAD_BYTES + 1];
        assert_eq!(
            normalize_json(&payload),
            Err(NormalizationError::PayloadTooLarge)
        );
    }
}
