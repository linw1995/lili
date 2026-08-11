use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

pub const ACTIONS_SCHEMA_VERSION: u16 = 1;
pub const DEFAULT_ACTION_TIMEOUT_MS: u64 = 10_000;
pub const MAX_ACTION_TIMEOUT_MS: u64 = 120_000;
pub const DEFAULT_ACTION_DEBOUNCE_MS: u64 = 250;
pub const MAX_ACTION_DEBOUNCE_MS: u64 = 60_000;
pub const DEFAULT_GLOBAL_CONCURRENCY: usize = 4;
pub const MAX_GLOBAL_CONCURRENCY: usize = 16;
pub const MAX_ACTION_QUEUE_CAPACITY: usize = 64;

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
    CodexHome,
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
policy = "codex_home"

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
            WorkingDirectoryPolicy::CodexHome
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
}
