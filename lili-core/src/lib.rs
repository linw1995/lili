mod diagnostics;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use diagnostics::{DiagnosticPrivacy, Redacted, diagnostic_privacy};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PetId(String);

impl PetId {
    pub fn parse(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        (!value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
        .then_some(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetLifecycleState {
    #[default]
    Idle,
    #[serde(alias = "running")]
    ActivityReminder,
    Review,
    Failed,
    Waiting,
}

impl PetLifecycleState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::ActivityReminder => "activity-reminder",
            Self::Review => "review",
            Self::Failed => "failed",
            Self::Waiting => "waiting",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetNotificationKind {
    Attention,
    Completion,
    Failure,
}

impl PetNotificationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Attention => "attention",
            Self::Completion => "completion",
            Self::Failure => "failure",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetNotificationPresentation {
    pub activation_id: String,
    pub kind: PetNotificationKind,
    pub project_label: Option<String>,
    pub summary: String,
    pub summary_truncated: bool,
    pub summary_redacted: bool,
    pub occurred_at_ms: u64,
    pub unread: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetActionFeedbackKind {
    Success,
    Failure,
    Busy,
}

impl PetActionFeedbackKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Busy => "busy",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetActionFeedbackPresentation {
    pub action_id: String,
    pub kind: PetActionFeedbackKind,
    pub message: String,
    pub occurred_at_ms: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetPresentationState {
    pub revision: u64,
    pub lifecycle: PetLifecycleState,
    pub pet_asset_id: Option<String>,
    pub pet_label: String,
    pub unread_notification_count: usize,
    pub notifications: Vec<PetNotificationPresentation>,
    #[serde(default)]
    pub action_feedback: Option<PetActionFeedbackPresentation>,
    pub reduced_motion: bool,
}
