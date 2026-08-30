mod execution;
mod loading;
mod supervisor;

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const ACTIONS_SCHEMA_VERSION: u16 = 1;
pub const DEFAULT_ACTION_TIMEOUT_MS: u64 = 10_000;
pub const MAX_ACTION_TIMEOUT_MS: u64 = 120_000;
pub const DEFAULT_ACTION_DEBOUNCE_MS: u64 = 250;
pub const MAX_ACTION_DEBOUNCE_MS: u64 = 60_000;
pub const DEFAULT_GLOBAL_CONCURRENCY: usize = 4;
pub const MAX_GLOBAL_CONCURRENCY: usize = 16;
pub const MAX_ACTION_QUEUE_CAPACITY: usize = 64;
pub const INTERACTION_CONTEXT_VERSION: u16 = 1;
pub const MAX_INTERACTION_CONTEXT_BYTES: usize = 16 * 1024;

pub use execution::{ActionSpawnError, SpawnedAction, spawn_action};
pub use loading::{
    ActionDiagnostic, ActionDiagnosticCode, ActionLoadContext, EffectiveActionView,
    EffectiveActionsView, LoadedAction, LoadedActions, MAX_ACTION_CONFIG_BYTES, MAX_ACTION_ENTRIES,
    action_config_path, load_actions_file, load_actions_str,
};
pub use supervisor::{
    ActionAuditEntry, ActionExecutionOutcome, ActionExecutionResult, ActionSupervisor,
    CapturedOutput, MAX_ACTION_AUDIT_ENTRIES, MAX_ACTION_OUTPUT_BYTES,
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionTrigger {
    PetClick,
    PetDoubleClick,
    NotificationActivate,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationFilterKind {
    Attention,
    Completion,
    Failure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetLifecycleSnapshotV1 {
    Idle,
    #[serde(rename = "running", alias = "activity_reminder")]
    ActivityReminder,
    Review,
    Failed,
    Waiting,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PetSnapshotV1 {
    pub pet_id: String,
    pub label: String,
    pub lifecycle: PetLifecycleSnapshotV1,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayEventSummaryV1 {
    pub text: String,
    pub truncated: bool,
    pub redacted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSnapshotV1 {
    pub notification_id: String,
    pub event_id: String,
    pub provider: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub kind: NotificationFilterKind,
    pub occurred_at_ms: u64,
    pub project_label: Option<String>,
    pub summary: Option<DisplayEventSummaryV1>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionContextV1 {
    pub version: u16,
    pub interaction_id: Uuid,
    pub accepted_at_ms: u64,
    pub trigger: InteractionTrigger,
    pub pet: PetSnapshotV1,
    pub notification: Option<NotificationSnapshotV1>,
}

impl InteractionContextV1 {
    pub fn for_pet(
        interaction_id: Uuid,
        accepted_at_ms: u64,
        trigger: InteractionTrigger,
        pet: PetSnapshotV1,
    ) -> Option<Self> {
        matches!(
            trigger,
            InteractionTrigger::PetClick | InteractionTrigger::PetDoubleClick
        )
        .then_some(Self {
            version: INTERACTION_CONTEXT_VERSION,
            interaction_id,
            accepted_at_ms,
            trigger,
            pet,
            notification: None,
        })
    }

    pub fn for_notification(
        interaction_id: Uuid,
        accepted_at_ms: u64,
        pet: PetSnapshotV1,
        notification: NotificationSnapshotV1,
    ) -> Self {
        Self {
            version: INTERACTION_CONTEXT_VERSION,
            interaction_id,
            accepted_at_ms,
            trigger: InteractionTrigger::NotificationActivate,
            pet,
            notification: Some(notification),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum InteractionContextError {
    #[error("interaction context exceeds its byte limit")]
    TooLarge,
    #[error("interaction context is malformed")]
    Malformed,
    #[error("interaction context violates schema invariants")]
    Invalid,
}

pub fn decode_interaction_context(
    payload: &[u8],
) -> Result<InteractionContextV1, InteractionContextError> {
    if payload.len() > MAX_INTERACTION_CONTEXT_BYTES {
        return Err(InteractionContextError::TooLarge);
    }
    let context: InteractionContextV1 =
        serde_json::from_slice(payload).map_err(|_| InteractionContextError::Malformed)?;
    if context.version != INTERACTION_CONTEXT_VERSION
        || context.pet.pet_id.is_empty()
        || context.pet.pet_id.len() > 128
        || context.pet.label.len() > 128
        || context
            .pet
            .pet_id
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'-' | b'_' | b'.'))
        || match context.trigger {
            InteractionTrigger::NotificationActivate => context.notification.is_none(),
            InteractionTrigger::PetClick | InteractionTrigger::PetDoubleClick => {
                context.notification.is_some()
            }
        }
    {
        return Err(InteractionContextError::Invalid);
    }
    Ok(context)
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct EventFilterV1 {
    pub notification_kinds: Vec<NotificationFilterKind>,
    pub providers: Vec<String>,
    pub project_labels: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkingDirectoryPolicy {
    #[default]
    Application,
    ApplicationData,
    Explicit,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct WorkingDirectoryV1 {
    pub policy: WorkingDirectoryPolicy,
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct EnvironmentV1 {
    pub allow: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConcurrencyMode {
    #[default]
    Reject,
    Queue,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct ConcurrencyV1 {
    pub mode: ConcurrencyMode,
    pub max_parallel: usize,
    pub queue_capacity: usize,
}

impl Default for ConcurrencyV1 {
    fn default() -> Self {
        Self {
            mode: ConcurrencyMode::Reject,
            max_parallel: 1,
            queue_capacity: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ActionConfigV1 {
    pub id: String,
    pub trigger: InteractionTrigger,
    #[serde(default)]
    pub filters: EventFilterV1,
    pub command: Vec<String>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
    #[serde(default)]
    pub concurrency: ConcurrencyV1,
    #[serde(default)]
    pub working_directory: WorkingDirectoryV1,
    #[serde(default)]
    pub environment: EnvironmentV1,
}

const fn default_timeout_ms() -> u64 {
    DEFAULT_ACTION_TIMEOUT_MS
}

const fn default_debounce_ms() -> u64 {
    DEFAULT_ACTION_DEBOUNCE_MS
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ActionsFileV1 {
    pub version: u16,
    #[serde(default, rename = "action")]
    pub actions: Vec<ActionConfigV1>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionSummary {
    pub id: String,
    pub trigger: InteractionTrigger,
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_v1_uses_explicit_argv_and_bounded_policy_fields() {
        let file: ActionsFileV1 = toml::from_str(
            r#"
version = 1

[[action]]
id = "open-session"
trigger = "notification_activate"
command = ["/usr/bin/example", "--mode", "desktop"]
timeout_ms = 5000
debounce_ms = 400

[action.filters]
notification_kinds = ["attention", "failure"]
providers = ["codex"]
project_labels = ["workspace"]

[action.concurrency]
mode = "queue"
max_parallel = 2
queue_capacity = 8

[action.working_directory]
policy = "application_data"

[action.environment.allow]
ACTION_MODE = "desktop"
"#,
        )
        .unwrap();

        assert_eq!(file.version, ACTIONS_SCHEMA_VERSION);
        assert_eq!(file.actions.len(), 1);
        let action = &file.actions[0];
        assert_eq!(action.trigger, InteractionTrigger::NotificationActivate);
        assert_eq!(action.command, ["/usr/bin/example", "--mode", "desktop"]);
        assert_eq!(action.concurrency.mode, ConcurrencyMode::Queue);
        assert_eq!(
            action.working_directory.policy,
            WorkingDirectoryPolicy::ApplicationData
        );
        assert_eq!(
            action
                .environment
                .allow
                .get("ACTION_MODE")
                .map(String::as_str),
            Some("desktop")
        );
    }

    #[test]
    fn action_defaults_are_stable_and_do_not_enable_string_commands() {
        let file: ActionsFileV1 = toml::from_str(
            r#"
version = 1

[[action]]
id = "wave"
trigger = "pet_click"
command = ["/usr/bin/example"]
"#,
        )
        .unwrap();
        let action = &file.actions[0];
        assert_eq!(action.timeout_ms, DEFAULT_ACTION_TIMEOUT_MS);
        assert_eq!(action.debounce_ms, DEFAULT_ACTION_DEBOUNCE_MS);
        assert_eq!(action.concurrency, ConcurrencyV1::default());
        assert!(
            toml::from_str::<ActionsFileV1>(
                r#"
version = 1
[[action]]
id = "unsafe"
trigger = "pet_click"
command = "echo unsafe"
"#
            )
            .is_err()
        );
    }

    #[test]
    fn distributed_actions_example_matches_schema() {
        let file: ActionsFileV1 =
            toml::from_str(include_str!("../../examples/actions.toml")).unwrap();

        assert_eq!(file.version, ACTIONS_SCHEMA_VERSION);
        assert_eq!(file.actions.len(), 3);
        assert_eq!(file.actions[0].trigger, InteractionTrigger::PetClick);
        assert_eq!(file.actions[1].trigger, InteractionTrigger::PetDoubleClick);
        assert_eq!(
            file.actions[2].trigger,
            InteractionTrigger::NotificationActivate
        );
    }

    #[test]
    fn interaction_context_v1_keeps_the_legacy_activity_wire_token() {
        let snapshot = PetSnapshotV1 {
            pet_id: "lili".to_owned(),
            label: "Lili".to_owned(),
            lifecycle: PetLifecycleSnapshotV1::ActivityReminder,
        };
        let encoded = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(encoded["lifecycle"], "running");

        let decoded: PetSnapshotV1 = serde_json::from_value(serde_json::json!({
            "petId": "lili",
            "label": "Lili",
            "lifecycle": "activity_reminder"
        }))
        .unwrap();
        assert_eq!(decoded.lifecycle, PetLifecycleSnapshotV1::ActivityReminder);
    }

    #[test]
    fn interaction_context_v1_serializes_only_bounded_display_safe_fields() {
        let context = InteractionContextV1::for_notification(
            Uuid::nil(),
            42,
            PetSnapshotV1 {
                pet_id: "lili".to_owned(),
                label: "Lili".to_owned(),
                lifecycle: PetLifecycleSnapshotV1::Review,
            },
            NotificationSnapshotV1 {
                notification_id: "notification-1".to_owned(),
                event_id: "event-1".to_owned(),
                provider: "codex".to_owned(),
                session_id: "session-1".to_owned(),
                turn_id: Some("turn-1".to_owned()),
                kind: NotificationFilterKind::Completion,
                occurred_at_ms: 40,
                project_label: Some("workspace".to_owned()),
                summary: Some(DisplayEventSummaryV1 {
                    text: "Finished".to_owned(),
                    truncated: false,
                    redacted: true,
                }),
            },
        );

        let encoded = serde_json::to_vec(&context).unwrap();
        assert!(encoded.len() <= MAX_INTERACTION_CONTEXT_BYTES);
        assert_eq!(decode_interaction_context(&encoded).unwrap(), context);
        let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(value["version"], INTERACTION_CONTEXT_VERSION);
        assert_eq!(value["trigger"], "notification_activate");
        assert_eq!(value["notification"]["sessionId"], "session-1");
        assert_eq!(value["notification"]["summary"]["redacted"], true);
        assert!(value.get("environment").is_none());
        assert!(value.get("credentials").is_none());
        assert!(value.get("rawPayload").is_none());
        assert_eq!(
            decode_interaction_context(&vec![b'x'; MAX_INTERACTION_CONTEXT_BYTES + 1]),
            Err(InteractionContextError::TooLarge)
        );
    }
}
