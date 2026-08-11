use std::fmt;

use serde::{Deserialize, Serialize};

use crate::SESSION_SCHEMA_VERSION;

const MAX_ID_BYTES: usize = 256;
pub const MAX_PROJECT_LABEL_CHARS: usize = 128;
pub const MAX_SUMMARY_CHARS: usize = 320;
pub const MAX_SOURCE_DISCRIMINATOR_CHARS: usize = 128;

macro_rules! bounded_identity {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
                let value = value.into();
                if value.is_empty()
                    || value.len() > MAX_ID_BYTES
                    || value.chars().any(char::is_control)
                {
                    return Err(IdentityError($label));
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdentityError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentityError(&'static str);

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} must contain 1 to {MAX_ID_BYTES} display-safe bytes",
            self.0
        )
    }
}

impl std::error::Error for IdentityError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DisplayValueError(&'static str);

impl fmt::Display for DisplayValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} is not display-safe", self.0)
    }
}

impl std::error::Error for DisplayValueError {}

bounded_identity!(ProviderId, "provider identity");
bounded_identity!(SessionId, "session identity");
bounded_identity!(TurnId, "turn identity");
bounded_identity!(EventId, "event identity");
bounded_identity!(NotificationId, "notification identity");

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilitiesInputV1 {
    #[serde(default)]
    pub reports_active: bool,
    #[serde(default)]
    pub reports_attention: bool,
    #[serde(default)]
    pub reports_completion: bool,
    #[serde(default)]
    pub reports_failure: bool,
    #[serde(default)]
    pub reports_resolution: bool,
    #[serde(default)]
    pub reports_session_end: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProjectInputV1 {
    pub label: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInputV1 {
    pub version: u16,
    pub provider: Option<String>,
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    pub event_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub occurred_at_ms: Option<u64>,
    pub project: Option<ProviderProjectInputV1>,
    pub summary: Option<String>,
    #[serde(default)]
    pub capabilities: ProviderCapabilitiesInputV1,
    pub source_discriminator: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    SessionStarted,
    TurnStarted,
    AttentionRequired,
    AttentionResolved,
    TurnCompleted,
    TurnFailed,
    SessionEnded,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayProjectContext {
    label: String,
}

impl DisplayProjectContext {
    pub fn parse(label: impl Into<String>) -> Result<Self, DisplayValueError> {
        let label = label.into();
        if label.is_empty()
            || label.chars().count() > MAX_PROJECT_LABEL_CHARS
            || label.chars().any(char::is_control)
        {
            return Err(DisplayValueError("project label"));
        }
        Ok(Self { label })
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplaySummary {
    text: String,
    truncated: bool,
    redacted: bool,
}

impl DisplaySummary {
    pub fn parse(
        text: impl Into<String>,
        truncated: bool,
        redacted: bool,
    ) -> Result<Self, DisplayValueError> {
        let text = text.into();
        if text.is_empty()
            || text.chars().count() > MAX_SUMMARY_CHARS
            || text
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
        {
            return Err(DisplayValueError("summary"));
        }
        Ok(Self {
            text,
            truncated,
            redacted,
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub const fn was_truncated(&self) -> bool {
        self.truncated
    }

    pub const fn was_redacted(&self) -> bool {
        self.redacted
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCapabilities {
    pub reports_active: bool,
    pub reports_attention: bool,
    pub reports_completion: bool,
    pub reports_failure: bool,
    pub reports_resolution: bool,
    pub reports_session_end: bool,
}

impl From<ProviderCapabilitiesInputV1> for SourceCapabilities {
    fn from(input: ProviderCapabilitiesInputV1) -> Self {
        Self {
            reports_active: input.reports_active,
            reports_attention: input.reports_attention,
            reports_completion: input.reports_completion,
            reports_failure: input.reports_failure,
            reports_resolution: input.reports_resolution,
            reports_session_end: input.reports_session_end,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedSessionEvent {
    pub version: u16,
    pub event_id: EventId,
    pub provider: ProviderId,
    pub event_type: SessionEventKind,
    pub session_id: SessionId,
    pub turn_id: Option<TurnId>,
    pub occurred_at_ms: u64,
    pub project: Option<DisplayProjectContext>,
    pub summary: Option<DisplaySummary>,
    pub capabilities: SourceCapabilities,
    pub source_discriminator: String,
}

impl NormalizedSessionEvent {
    pub const fn current_version() -> u16 {
        SESSION_SCHEMA_VERSION
    }

    pub fn validate(&self) -> Result<(), NormalizedEventValidationError> {
        if self.version != SESSION_SCHEMA_VERSION {
            return Err(NormalizedEventValidationError::UnsupportedVersion);
        }
        if !matches!(
            self.event_type,
            SessionEventKind::SessionStarted | SessionEventKind::SessionEnded
        ) && self.turn_id.is_none()
        {
            return Err(NormalizedEventValidationError::MissingTurn);
        }
        if let Some(project) = &self.project {
            DisplayProjectContext::parse(project.label.clone())
                .map_err(|_| NormalizedEventValidationError::InvalidProject)?;
        }
        if let Some(summary) = &self.summary {
            DisplaySummary::parse(summary.text.clone(), summary.truncated, summary.redacted)
                .map_err(|_| NormalizedEventValidationError::InvalidSummary)?;
        }
        if self.source_discriminator.is_empty()
            || self.source_discriminator.chars().count() > MAX_SOURCE_DISCRIMINATOR_CHARS
            || self.source_discriminator.chars().any(char::is_control)
        {
            return Err(NormalizedEventValidationError::InvalidSource);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NormalizedEventValidationError {
    UnsupportedVersion,
    MissingTurn,
    InvalidProject,
    InvalidSummary,
    InvalidSource,
}

impl fmt::Display for NormalizedEventValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedVersion => "normalized event version is unsupported",
            Self::MissingTurn => "normalized turn event is missing a turn identity",
            Self::InvalidProject => "normalized event project is invalid",
            Self::InvalidSummary => "normalized event summary is invalid",
            Self::InvalidSource => "normalized event source is invalid",
        })
    }
}

impl std::error::Error for NormalizedEventValidationError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    Attention,
    Completion,
    Failure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationState {
    Unread,
    Acknowledged,
    Resolved,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub id: NotificationId,
    pub provider: ProviderId,
    pub event_id: EventId,
    pub session_id: SessionId,
    pub turn_id: Option<TurnId>,
    pub kind: NotificationKind,
    pub state: NotificationState,
    pub occurred_at_ms: u64,
    pub project: Option<DisplayProjectContext>,
    pub summary: Option<DisplaySummary>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    #[default]
    Idle,
    Active,
    Attention,
    Completed,
    Failed,
    Ended,
}

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentationState {
    #[default]
    Idle,
    Running,
    Review,
    Failed,
    Waiting,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub provider: ProviderId,
    pub id: SessionId,
    pub current_turn_id: Option<TurnId>,
    pub phase: SessionPhase,
    pub project: Option<DisplayProjectContext>,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionViewSnapshot {
    pub revision: u64,
    pub presentation: PresentationState,
    pub sessions: Vec<SessionSummary>,
    pub notifications: Vec<Notification>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_event_errors_have_safe_public_messages() {
        let errors = [
            NormalizedEventValidationError::UnsupportedVersion,
            NormalizedEventValidationError::MissingTurn,
            NormalizedEventValidationError::InvalidProject,
            NormalizedEventValidationError::InvalidSummary,
            NormalizedEventValidationError::InvalidSource,
        ];

        for error in errors {
            assert!(error.to_string().starts_with("normalized "));
        }
    }

    #[test]
    fn identities_reject_empty_oversized_and_control_values() {
        assert!(SessionId::parse("").is_err());
        assert!(TurnId::parse("x".repeat(MAX_ID_BYTES + 1)).is_err());
        assert!(EventId::parse("event\nidentity").is_err());
        assert_eq!(ProviderId::parse("codex").unwrap().as_str(), "codex");
    }

    #[test]
    fn display_values_enforce_their_public_bounds() {
        assert!(DisplayProjectContext::parse("workspace").is_ok());
        assert!(DisplayProjectContext::parse("x".repeat(MAX_PROJECT_LABEL_CHARS + 1)).is_err());
        assert!(DisplaySummary::parse("done", false, false).is_ok());
        assert!(DisplaySummary::parse("x".repeat(MAX_SUMMARY_CHARS + 1), true, false).is_err());
    }

    #[test]
    fn provider_input_ignores_additive_fields() {
        let input: ProviderInputV1 = serde_json::from_str(
            r#"{"version":1,"provider":"codex","type":"turn_completed","sessionId":"session-1","futureField":{"nested":true}}"#,
        )
        .unwrap();
        assert_eq!(input.version, SESSION_SCHEMA_VERSION);
        assert_eq!(input.provider.as_deref(), Some("codex"));
        assert_eq!(input.event_type.as_deref(), Some("turn_completed"));
        assert_eq!(input.session_id.as_deref(), Some("session-1"));
    }

    #[test]
    fn view_snapshot_uses_stable_external_field_names() {
        let value = serde_json::to_value(SessionViewSnapshot::default()).unwrap();
        assert_eq!(value["revision"], 0);
        assert_eq!(value["presentation"], "idle");
        assert!(value["sessions"].is_array());
        assert!(value["notifications"].is_array());
    }
}
