use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{
    ACTIONS_SCHEMA_VERSION, ActionConfigV1, ActionSummary, ConcurrencyMode, EventFilterV1,
    InteractionTrigger, MAX_ACTION_DEBOUNCE_MS, MAX_ACTION_QUEUE_CAPACITY, MAX_ACTION_TIMEOUT_MS,
    MAX_GLOBAL_CONCURRENCY, WorkingDirectoryPolicy,
};

pub const MAX_ACTION_CONFIG_BYTES: usize = 256 * 1024;
pub const MAX_ACTION_ENTRIES: usize = 128;
const MAX_ACTION_ID_BYTES: usize = 128;
const MAX_COMMAND_PART_BYTES: usize = 16 * 1024;
const MAX_ENVIRONMENT_ENTRIES: usize = 64;
const MAX_ENVIRONMENT_VALUE_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionLoadContext {
    application_dir: PathBuf,
    codex_home: PathBuf,
    executable_search_path: Vec<PathBuf>,
}

impl ActionLoadContext {
    pub fn new(
        application_dir: impl Into<PathBuf>,
        codex_home: impl Into<PathBuf>,
        executable_search_path: Vec<PathBuf>,
    ) -> Self {
        Self {
            application_dir: application_dir.into(),
            codex_home: codex_home.into(),
            executable_search_path,
        }
    }

    pub fn for_codex_home(codex_home: impl Into<PathBuf>) -> Self {
        let application_dir = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        let executable_search_path = std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect())
            .unwrap_or_default();
        Self::new(application_dir, codex_home, executable_search_path)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionDiagnosticCode {
    Missing,
    InvalidFile,
    TooLarge,
    MalformedDocument,
    UnsupportedVersion,
    TooManyEntries,
    MalformedEntry,
    InvalidIdentifier,
    DuplicateIdentifier,
    EmptyCommand,
    InvalidCommand,
    ExecutableNotFound,
    InvalidTimeout,
    InvalidDebounce,
    InvalidConcurrency,
    InvalidWorkingDirectory,
    InvalidEnvironment,
    InvalidFilter,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionDiagnostic {
    pub entry_index: Option<usize>,
    pub action_id: Option<String>,
    pub code: ActionDiagnosticCode,
    pub message: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveActionView {
    pub entry_index: usize,
    pub id: Option<String>,
    pub trigger: Option<InteractionTrigger>,
    pub enabled: bool,
    pub executable_resolved: bool,
    pub argument_count: usize,
    pub timeout_ms: Option<u64>,
    pub debounce_ms: Option<u64>,
    pub concurrency_mode: Option<ConcurrencyMode>,
    pub max_parallel: Option<usize>,
    pub queue_capacity: Option<usize>,
    pub working_directory_policy: Option<WorkingDirectoryPolicy>,
    pub environment_names: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveActionsView {
    pub schema_version: Option<u16>,
    pub actions: Vec<EffectiveActionView>,
    pub diagnostics: Vec<ActionDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedAction {
    id: String,
    trigger: InteractionTrigger,
    filters: EventFilterV1,
    executable: PathBuf,
    arguments: Vec<OsString>,
    timeout_ms: u64,
    debounce_ms: u64,
    concurrency_mode: ConcurrencyMode,
    max_parallel: usize,
    queue_capacity: usize,
    working_directory: PathBuf,
    working_directory_policy: WorkingDirectoryPolicy,
    environment: BTreeMap<String, String>,
}

impl LoadedAction {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub const fn trigger(&self) -> InteractionTrigger {
        self.trigger
    }

    pub const fn filters(&self) -> &EventFilterV1 {
        &self.filters
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    pub const fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    pub const fn debounce_ms(&self) -> u64 {
        self.debounce_ms
    }

    pub const fn concurrency_mode(&self) -> ConcurrencyMode {
        self.concurrency_mode
    }

    pub const fn max_parallel(&self) -> usize {
        self.max_parallel
    }

    pub const fn queue_capacity(&self) -> usize {
        self.queue_capacity
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    pub const fn working_directory_policy(&self) -> WorkingDirectoryPolicy {
        self.working_directory_policy
    }

    pub fn environment(&self) -> &BTreeMap<String, String> {
        &self.environment
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadedActions {
    enabled: Vec<LoadedAction>,
    effective: EffectiveActionsView,
}

impl LoadedActions {
    pub fn enabled(&self) -> &[LoadedAction] {
        &self.enabled
    }

    pub const fn effective(&self) -> &EffectiveActionsView {
        &self.effective
    }

    pub fn summaries(&self) -> Vec<ActionSummary> {
        self.effective
            .actions
            .iter()
            .filter_map(|action| {
                Some(ActionSummary {
                    id: action.id.clone()?,
                    trigger: action.trigger?,
                    enabled: action.enabled,
                })
            })
            .collect()
    }
}

pub fn action_config_path(codex_home: &Path) -> PathBuf {
    codex_home.join("lili").join("actions.toml")
}

pub fn load_actions_file(path: &Path, context: &ActionLoadContext) -> LoadedActions {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return global_failure(ActionDiagnosticCode::InvalidFile);
        }
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return LoadedActions {
                enabled: Vec::new(),
                effective: EffectiveActionsView {
                    schema_version: None,
                    actions: Vec::new(),
                    diagnostics: Vec::new(),
                },
            };
        }
        Err(_) => return global_failure(ActionDiagnosticCode::InvalidFile),
    };
    if metadata.len() > MAX_ACTION_CONFIG_BYTES as u64 {
        return global_failure(ActionDiagnosticCode::TooLarge);
    }
    match fs::read_to_string(path) {
        Ok(contents) if contents.len() <= MAX_ACTION_CONFIG_BYTES => {
            load_actions_str(&contents, context)
        }
        Ok(_) => global_failure(ActionDiagnosticCode::TooLarge),
        Err(_) => global_failure(ActionDiagnosticCode::InvalidFile),
    }
}

pub fn load_actions_str(contents: &str, context: &ActionLoadContext) -> LoadedActions {
    if contents.len() > MAX_ACTION_CONFIG_BYTES {
        return global_failure(ActionDiagnosticCode::TooLarge);
    }
    let Ok(table) = toml::from_str::<toml::Table>(contents) else {
        return global_failure(ActionDiagnosticCode::MalformedDocument);
    };
    let schema_version = table
        .get("version")
        .and_then(toml::Value::as_integer)
        .and_then(|version| u16::try_from(version).ok());
    if schema_version != Some(ACTIONS_SCHEMA_VERSION) {
        let mut loaded = global_failure(ActionDiagnosticCode::UnsupportedVersion);
        loaded.effective.schema_version = schema_version;
        return loaded;
    }
    let entries = table
        .get("action")
        .and_then(toml::Value::as_array)
        .cloned()
        .unwrap_or_default();
    if entries.len() > MAX_ACTION_ENTRIES {
        let mut loaded = global_failure(ActionDiagnosticCode::TooManyEntries);
        loaded.effective.schema_version = schema_version;
        return loaded;
    }

    let mut enabled = Vec::new();
    let mut actions = Vec::with_capacity(entries.len());
    let mut diagnostics = Vec::new();
    let mut identifiers = BTreeSet::new();
    for (entry_index, value) in entries.into_iter().enumerate() {
        let hinted_id = safe_hinted_id(&value);
        let parsed = value.try_into::<ActionConfigV1>();
        let Ok(config) = parsed else {
            diagnostics.push(diagnostic(
                entry_index,
                hinted_id.clone(),
                ActionDiagnosticCode::MalformedEntry,
            ));
            actions.push(disabled_view(entry_index, hinted_id));
            continue;
        };
        let id = safe_identifier(&config.id);
        let Some(id) = id else {
            diagnostics.push(diagnostic(
                entry_index,
                None,
                ActionDiagnosticCode::InvalidIdentifier,
            ));
            actions.push(disabled_view(entry_index, None));
            continue;
        };
        if !identifiers.insert(id.clone()) {
            diagnostics.push(diagnostic(
                entry_index,
                Some(id.clone()),
                ActionDiagnosticCode::DuplicateIdentifier,
            ));
            actions.push(disabled_config_view(entry_index, id, &config));
            continue;
        }
        match validate_action(id.clone(), config, context) {
            Ok(action) => {
                actions.push(enabled_view(entry_index, &action));
                enabled.push(action);
            }
            Err(code) => {
                diagnostics.push(diagnostic(entry_index, Some(id.clone()), code));
                actions.push(disabled_view(entry_index, Some(id)));
            }
        }
    }
    LoadedActions {
        enabled,
        effective: EffectiveActionsView {
            schema_version,
            actions,
            diagnostics,
        },
    }
}

fn validate_action(
    id: String,
    config: ActionConfigV1,
    context: &ActionLoadContext,
) -> Result<LoadedAction, ActionDiagnosticCode> {
    if config.command.is_empty() {
        return Err(ActionDiagnosticCode::EmptyCommand);
    }
    if config.command.iter().any(|part| {
        part.is_empty()
            || part.len() > MAX_COMMAND_PART_BYTES
            || part.as_bytes().contains(&0)
            || part.chars().any(char::is_control)
    }) {
        return Err(ActionDiagnosticCode::InvalidCommand);
    }
    let executable = resolve_executable(&config.command[0], context)
        .ok_or(ActionDiagnosticCode::ExecutableNotFound)?;
    if !(1..=MAX_ACTION_TIMEOUT_MS).contains(&config.timeout_ms) {
        return Err(ActionDiagnosticCode::InvalidTimeout);
    }
    if config.debounce_ms > MAX_ACTION_DEBOUNCE_MS {
        return Err(ActionDiagnosticCode::InvalidDebounce);
    }
    if !(1..=MAX_GLOBAL_CONCURRENCY).contains(&config.concurrency.max_parallel)
        || config.concurrency.queue_capacity > MAX_ACTION_QUEUE_CAPACITY
        || match config.concurrency.mode {
            ConcurrencyMode::Reject => config.concurrency.queue_capacity != 0,
            ConcurrencyMode::Queue => config.concurrency.queue_capacity == 0,
        }
    {
        return Err(ActionDiagnosticCode::InvalidConcurrency);
    }
    let working_directory = match config.working_directory.policy {
        WorkingDirectoryPolicy::Application if config.working_directory.path.is_none() => {
            context.application_dir.clone()
        }
        WorkingDirectoryPolicy::CodexHome if config.working_directory.path.is_none() => {
            context.codex_home.clone()
        }
        WorkingDirectoryPolicy::Explicit => config
            .working_directory
            .path
            .filter(|path| path.is_absolute() && path.is_dir())
            .ok_or(ActionDiagnosticCode::InvalidWorkingDirectory)?,
        _ => return Err(ActionDiagnosticCode::InvalidWorkingDirectory),
    };
    if !working_directory.is_dir() {
        return Err(ActionDiagnosticCode::InvalidWorkingDirectory);
    }
    if config.environment.allow.len() > MAX_ENVIRONMENT_ENTRIES
        || config.environment.allow.iter().any(|(name, value)| {
            !valid_environment_name(name)
                || value.len() > MAX_ENVIRONMENT_VALUE_BYTES
                || value.as_bytes().contains(&0)
        })
    {
        return Err(ActionDiagnosticCode::InvalidEnvironment);
    }
    if !valid_filters(&config.filters) {
        return Err(ActionDiagnosticCode::InvalidFilter);
    }
    Ok(LoadedAction {
        id,
        trigger: config.trigger,
        filters: config.filters,
        executable,
        arguments: config
            .command
            .into_iter()
            .skip(1)
            .map(OsString::from)
            .collect(),
        timeout_ms: config.timeout_ms,
        debounce_ms: config.debounce_ms,
        concurrency_mode: config.concurrency.mode,
        max_parallel: config.concurrency.max_parallel,
        queue_capacity: config.concurrency.queue_capacity,
        working_directory,
        working_directory_policy: config.working_directory.policy,
        environment: config.environment.allow,
    })
}

fn resolve_executable(executable: &str, context: &ActionLoadContext) -> Option<PathBuf> {
    let path = Path::new(executable);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    if path.components().count() > 1 {
        let candidate = context.application_dir.join(path);
        return candidate.is_file().then_some(candidate);
    }
    context
        .executable_search_path
        .iter()
        .map(|directory| directory.join(path))
        .find(|candidate| candidate.is_file())
}

fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn valid_filters(filters: &EventFilterV1) -> bool {
    filters.providers.iter().all(|value| {
        !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
    }) && filters.project_labels.iter().all(|value| {
        !value.is_empty() && value.chars().count() <= 128 && !value.chars().any(char::is_control)
    })
}

fn safe_identifier(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= MAX_ACTION_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
    .then(|| value.to_owned())
}

fn safe_hinted_id(value: &toml::Value) -> Option<String> {
    value
        .as_table()
        .and_then(|table| table.get("id"))
        .and_then(toml::Value::as_str)
        .and_then(safe_identifier)
}

fn enabled_view(entry_index: usize, action: &LoadedAction) -> EffectiveActionView {
    EffectiveActionView {
        entry_index,
        id: Some(action.id.clone()),
        trigger: Some(action.trigger),
        enabled: true,
        executable_resolved: true,
        argument_count: action.arguments.len(),
        timeout_ms: Some(action.timeout_ms),
        debounce_ms: Some(action.debounce_ms),
        concurrency_mode: Some(action.concurrency_mode),
        max_parallel: Some(action.max_parallel),
        queue_capacity: Some(action.queue_capacity),
        working_directory_policy: Some(action.working_directory_policy),
        environment_names: action.environment.keys().cloned().collect(),
    }
}

fn disabled_config_view(
    entry_index: usize,
    id: String,
    config: &ActionConfigV1,
) -> EffectiveActionView {
    EffectiveActionView {
        entry_index,
        id: Some(id),
        trigger: Some(config.trigger),
        enabled: false,
        executable_resolved: false,
        argument_count: config.command.len().saturating_sub(1),
        timeout_ms: Some(config.timeout_ms),
        debounce_ms: Some(config.debounce_ms),
        concurrency_mode: Some(config.concurrency.mode),
        max_parallel: Some(config.concurrency.max_parallel),
        queue_capacity: Some(config.concurrency.queue_capacity),
        working_directory_policy: Some(config.working_directory.policy),
        environment_names: config.environment.allow.keys().cloned().collect(),
    }
}

fn disabled_view(entry_index: usize, id: Option<String>) -> EffectiveActionView {
    EffectiveActionView {
        entry_index,
        id,
        trigger: None,
        enabled: false,
        executable_resolved: false,
        argument_count: 0,
        timeout_ms: None,
        debounce_ms: None,
        concurrency_mode: None,
        max_parallel: None,
        queue_capacity: None,
        working_directory_policy: None,
        environment_names: Vec::new(),
    }
}

fn global_failure(code: ActionDiagnosticCode) -> LoadedActions {
    LoadedActions {
        enabled: Vec::new(),
        effective: EffectiveActionsView {
            schema_version: None,
            actions: Vec::new(),
            diagnostics: vec![ActionDiagnostic {
                entry_index: None,
                action_id: None,
                code,
                message: diagnostic_message(code),
            }],
        },
    }
}

fn diagnostic(
    entry_index: usize,
    action_id: Option<String>,
    code: ActionDiagnosticCode,
) -> ActionDiagnostic {
    ActionDiagnostic {
        entry_index: Some(entry_index),
        action_id,
        code,
        message: diagnostic_message(code),
    }
}

const fn diagnostic_message(code: ActionDiagnosticCode) -> &'static str {
    match code {
        ActionDiagnosticCode::Missing => "action configuration is missing",
        ActionDiagnosticCode::InvalidFile => "action configuration is not a readable regular file",
        ActionDiagnosticCode::TooLarge => "action configuration exceeds its byte limit",
        ActionDiagnosticCode::MalformedDocument => "action configuration is malformed",
        ActionDiagnosticCode::UnsupportedVersion => "action configuration version is unsupported",
        ActionDiagnosticCode::TooManyEntries => "action configuration has too many entries",
        ActionDiagnosticCode::MalformedEntry => "action entry does not match the v1 schema",
        ActionDiagnosticCode::InvalidIdentifier => "action identifier is invalid",
        ActionDiagnosticCode::DuplicateIdentifier => "action identifier is duplicated",
        ActionDiagnosticCode::EmptyCommand => "action command is empty",
        ActionDiagnosticCode::InvalidCommand => "action argv is invalid",
        ActionDiagnosticCode::ExecutableNotFound => "action executable could not be resolved",
        ActionDiagnosticCode::InvalidTimeout => "action timeout is outside the allowed range",
        ActionDiagnosticCode::InvalidDebounce => "action debounce is outside the allowed range",
        ActionDiagnosticCode::InvalidConcurrency => "action concurrency policy is invalid",
        ActionDiagnosticCode::InvalidWorkingDirectory => {
            "action working directory policy is invalid"
        }
        ActionDiagnosticCode::InvalidEnvironment => "action environment allowlist is invalid",
        ActionDiagnosticCode::InvalidFilter => "action event filter is invalid",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> ActionLoadContext {
        let executable = std::env::current_exe().unwrap();
        ActionLoadContext::new(
            executable.parent().unwrap(),
            std::env::temp_dir(),
            vec![executable.parent().unwrap().to_path_buf()],
        )
    }

    fn executable_literal() -> String {
        format!("{:?}", std::env::current_exe().unwrap().to_string_lossy())
    }

    #[test]
    fn invalid_and_duplicate_entries_do_not_disable_other_actions() {
        let source = format!(
            r#"
version = 1

[[action]]
id = "valid"
trigger = "pet_click"
command = [{}]

[action.environment.allow]
VISIBLE_NAME = "secret-value"

[[action]]
id = "bad-timeout"
trigger = "pet_double_click"
command = [{}]
timeout_ms = 0

[[action]]
id = "valid"
trigger = "notification_activate"
command = [{}]

[[action]]
id = "bad-trigger"
trigger = "unsupported"
command = [{}]
"#,
            executable_literal(),
            executable_literal(),
            executable_literal(),
            executable_literal(),
        );
        let loaded = load_actions_str(&source, &context());
        assert_eq!(loaded.enabled().len(), 1);
        assert_eq!(loaded.enabled()[0].id(), "valid");
        assert_eq!(loaded.effective().actions.len(), 4);
        assert_eq!(loaded.effective().diagnostics.len(), 3);
        assert_eq!(
            loaded.effective().diagnostics[0].code,
            ActionDiagnosticCode::InvalidTimeout
        );
        assert_eq!(
            loaded.effective().diagnostics[1].code,
            ActionDiagnosticCode::DuplicateIdentifier
        );
        assert_eq!(
            loaded.effective().diagnostics[2].code,
            ActionDiagnosticCode::MalformedEntry
        );

        let serialized = serde_json::to_string(loaded.effective()).unwrap();
        assert!(serialized.contains("VISIBLE_NAME"));
        assert!(!serialized.contains("secret-value"));
        assert!(!serialized.contains(std::env::current_exe().unwrap().to_string_lossy().as_ref()));
    }

    #[test]
    fn unsupported_version_and_malformed_document_fail_closed() {
        let unsupported = load_actions_str("version = 2", &context());
        assert!(unsupported.enabled().is_empty());
        assert_eq!(
            unsupported.effective().diagnostics[0].code,
            ActionDiagnosticCode::UnsupportedVersion
        );
        let malformed = load_actions_str("version = [", &context());
        assert_eq!(
            malformed.effective().diagnostics[0].code,
            ActionDiagnosticCode::MalformedDocument
        );
    }
}
