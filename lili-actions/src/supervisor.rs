use std::{
    collections::{BTreeMap, VecDeque},
    io,
    process::ExitStatus,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::{Mutex, OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
    time::Instant,
};

use crate::{
    ConcurrencyMode, InteractionContextV1, InteractionTrigger, LoadedAction, LoadedActions,
    MAX_GLOBAL_CONCURRENCY, spawn_action,
};

pub const MAX_ACTION_OUTPUT_BYTES: usize = 16 * 1024;
pub const MAX_ACTION_AUDIT_ENTRIES: usize = 256;
const PROCESS_IO_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionExecutionOutcome {
    Succeeded,
    NonZeroExit,
    SpawnFailed,
    IoFailed,
    TimedOut,
    OutputOverflow,
    Debounced,
    Saturated,
    NotMatched,
    UnknownAction,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapturedOutput {
    bytes: Vec<u8>,
    truncated_bytes: u64,
}

impl CapturedOutput {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub const fn truncated_bytes(&self) -> u64 {
        self.truncated_bytes
    }

    pub fn overflowed(&self) -> bool {
        self.truncated_bytes > 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionExecutionResult {
    pub action_id: String,
    pub interaction_id: uuid::Uuid,
    pub trigger: InteractionTrigger,
    pub event_id: Option<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub outcome: ActionExecutionOutcome,
    pub exit_code: Option<i32>,
    pub stdout: CapturedOutput,
    pub stderr: CapturedOutput,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionAuditEntry {
    pub action_id: String,
    pub trigger: InteractionTrigger,
    pub event_id: Option<String>,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    pub outcome: ActionExecutionOutcome,
    pub exit_code: Option<i32>,
    pub stdout_captured_bytes: usize,
    pub stdout_truncated_bytes: u64,
    pub stderr_captured_bytes: usize,
    pub stderr_truncated_bytes: u64,
}

impl From<&ActionExecutionResult> for ActionAuditEntry {
    fn from(result: &ActionExecutionResult) -> Self {
        Self {
            action_id: result.action_id.clone(),
            trigger: result.trigger,
            event_id: result.event_id.clone(),
            started_at_ms: result.started_at_ms,
            finished_at_ms: result.finished_at_ms,
            outcome: result.outcome,
            exit_code: result.exit_code,
            stdout_captured_bytes: result.stdout.bytes.len(),
            stdout_truncated_bytes: result.stdout.truncated_bytes,
            stderr_captured_bytes: result.stderr.bytes.len(),
            stderr_truncated_bytes: result.stderr.truncated_bytes,
        }
    }
}

impl ActionExecutionResult {
    fn immediate(
        action_id: impl Into<String>,
        context: &InteractionContextV1,
        outcome: ActionExecutionOutcome,
    ) -> Self {
        let now_ms = unix_time_ms();
        Self {
            action_id: action_id.into(),
            interaction_id: context.interaction_id,
            trigger: context.trigger,
            event_id: context
                .notification
                .as_ref()
                .map(|notification| notification.event_id.clone()),
            started_at_ms: now_ms,
            finished_at_ms: now_ms,
            outcome,
            exit_code: None,
            stdout: CapturedOutput::default(),
            stderr: CapturedOutput::default(),
        }
    }
}

#[derive(Clone)]
pub struct ActionSupervisor {
    actions: Arc<BTreeMap<String, Arc<ActionRuntime>>>,
    global: Arc<Semaphore>,
    audit: Arc<Mutex<VecDeque<ActionAuditEntry>>>,
}

impl ActionSupervisor {
    pub fn new(actions: LoadedActions, global_concurrency: usize) -> Option<Self> {
        if !(1..=MAX_GLOBAL_CONCURRENCY).contains(&global_concurrency) {
            return None;
        }
        let actions = actions
            .enabled()
            .iter()
            .cloned()
            .map(|action| {
                let id = action.id().to_owned();
                (id, Arc::new(ActionRuntime::new(action)))
            })
            .collect();
        Some(Self {
            actions: Arc::new(actions),
            global: Arc::new(Semaphore::new(global_concurrency)),
            audit: Arc::new(Mutex::new(VecDeque::with_capacity(
                MAX_ACTION_AUDIT_ENTRIES,
            ))),
        })
    }

    pub fn matching_action_ids(&self, context: &InteractionContextV1) -> Vec<String> {
        self.actions
            .values()
            .filter(|runtime| matches_context(&runtime.action, context))
            .map(|runtime| runtime.action.id().to_owned())
            .collect()
    }

    pub async fn execute(
        &self,
        action_id: &str,
        context: &InteractionContextV1,
    ) -> ActionExecutionResult {
        let result = self.execute_unrecorded(action_id, context).await;
        if !matches!(
            result.outcome,
            ActionExecutionOutcome::NotMatched | ActionExecutionOutcome::UnknownAction
        ) {
            self.record_audit(&result).await;
        }
        result
    }

    pub async fn audit_snapshot(&self) -> Vec<ActionAuditEntry> {
        self.audit.lock().await.iter().cloned().collect()
    }

    async fn execute_unrecorded(
        &self,
        action_id: &str,
        context: &InteractionContextV1,
    ) -> ActionExecutionResult {
        let Some(runtime) = self.actions.get(action_id).cloned() else {
            return ActionExecutionResult::immediate(
                action_id,
                context,
                ActionExecutionOutcome::UnknownAction,
            );
        };
        if !matches_context(&runtime.action, context) {
            return ActionExecutionResult::immediate(
                action_id,
                context,
                ActionExecutionOutcome::NotMatched,
            );
        }
        let Some(_admission) = runtime.admit() else {
            return ActionExecutionResult::immediate(
                action_id,
                context,
                ActionExecutionOutcome::Saturated,
            );
        };
        if !runtime.accept_debounce().await {
            return ActionExecutionResult::immediate(
                action_id,
                context,
                ActionExecutionOutcome::Debounced,
            );
        }
        let Ok(_action_slot) = runtime.running.clone().acquire_owned().await else {
            return ActionExecutionResult::immediate(
                action_id,
                context,
                ActionExecutionOutcome::Saturated,
            );
        };
        let Ok(_global_slot) = self.global.clone().acquire_owned().await else {
            return ActionExecutionResult::immediate(
                action_id,
                context,
                ActionExecutionOutcome::Saturated,
            );
        };
        run_action(&runtime.action, context).await
    }

    async fn record_audit(&self, result: &ActionExecutionResult) {
        let mut audit = self.audit.lock().await;
        if audit.len() == MAX_ACTION_AUDIT_ENTRIES {
            audit.pop_front();
        }
        audit.push_back(ActionAuditEntry::from(result));
    }
}

struct ActionRuntime {
    action: LoadedAction,
    admission: Arc<Semaphore>,
    running: Arc<Semaphore>,
    last_accepted: Mutex<Option<Instant>>,
}

impl ActionRuntime {
    fn new(action: LoadedAction) -> Self {
        let admission_capacity = match action.concurrency_mode() {
            ConcurrencyMode::Reject => action.max_parallel(),
            ConcurrencyMode::Queue => action
                .max_parallel()
                .saturating_add(action.queue_capacity()),
        };
        Self {
            running: Arc::new(Semaphore::new(action.max_parallel())),
            admission: Arc::new(Semaphore::new(admission_capacity)),
            action,
            last_accepted: Mutex::new(None),
        }
    }

    fn admit(&self) -> Option<OwnedSemaphorePermit> {
        self.admission.clone().try_acquire_owned().ok()
    }

    async fn accept_debounce(&self) -> bool {
        let now = Instant::now();
        let mut last_accepted = self.last_accepted.lock().await;
        if last_accepted.is_some_and(|last| {
            now.duration_since(last) < Duration::from_millis(self.action.debounce_ms())
        }) {
            return false;
        }
        *last_accepted = Some(now);
        true
    }
}

fn matches_context(action: &LoadedAction, context: &InteractionContextV1) -> bool {
    if action.trigger() != context.trigger {
        return false;
    }
    let filters = action.filters();
    if filters.notification_kinds.is_empty()
        && filters.providers.is_empty()
        && filters.project_labels.is_empty()
    {
        return true;
    }
    let Some(notification) = context.notification.as_ref() else {
        return false;
    };
    (filters.notification_kinds.is_empty()
        || filters.notification_kinds.contains(&notification.kind))
        && (filters.providers.is_empty() || filters.providers.contains(&notification.provider))
        && (filters.project_labels.is_empty()
            || notification
                .project_label
                .as_ref()
                .is_some_and(|project| filters.project_labels.contains(project)))
}

async fn run_action(
    action: &LoadedAction,
    context: &InteractionContextV1,
) -> ActionExecutionResult {
    let started_at_ms = unix_time_ms();
    let result = spawn_action(action, context).await;
    let Ok(mut spawned) = result else {
        return completed_result(
            action,
            context,
            started_at_ms,
            ActionExecutionOutcome::SpawnFailed,
            None,
            CapturedOutput::default(),
            CapturedOutput::default(),
        );
    };
    let stdout = spawned.child_mut().stdout.take();
    let stderr = spawned.child_mut().stderr.take();
    let stdin_task = spawned.take_stdin_task();
    let stdout_task = tokio::spawn(capture_output(stdout));
    let stderr_task = tokio::spawn(capture_output(stderr));
    let timeout = Duration::from_millis(action.timeout_ms());
    let status = tokio::time::timeout(timeout, spawned.child_mut().wait()).await;

    let (timed_out, wait_failed, status) = match status {
        Ok(Ok(status)) => (false, false, Some(status)),
        Ok(Err(_)) => {
            let _ = spawned.terminate_tree().await;
            (false, true, None)
        }
        Err(_) => {
            let _ = spawned.terminate_tree().await;
            (true, false, None)
        }
    };
    spawned.mark_finished();
    let stdin_failed = join_stdin(stdin_task).await.is_none();
    let stdout = join_capture(stdout_task).await;
    let stderr = join_capture(stderr_task).await;
    let capture_failed = stdout.is_none() || stderr.is_none();
    let stdout = stdout.unwrap_or_default();
    let stderr = stderr.unwrap_or_default();
    let outcome = if timed_out {
        ActionExecutionOutcome::TimedOut
    } else if stdin_failed || capture_failed || wait_failed || status.is_none() {
        ActionExecutionOutcome::IoFailed
    } else if stdout.overflowed() || stderr.overflowed() {
        ActionExecutionOutcome::OutputOverflow
    } else if status.as_ref().is_some_and(ExitStatus::success) {
        ActionExecutionOutcome::Succeeded
    } else {
        ActionExecutionOutcome::NonZeroExit
    };
    completed_result(
        action,
        context,
        started_at_ms,
        outcome,
        status.as_ref().and_then(ExitStatus::code),
        stdout,
        stderr,
    )
}

fn completed_result(
    action: &LoadedAction,
    context: &InteractionContextV1,
    started_at_ms: u64,
    outcome: ActionExecutionOutcome,
    exit_code: Option<i32>,
    stdout: CapturedOutput,
    stderr: CapturedOutput,
) -> ActionExecutionResult {
    ActionExecutionResult {
        action_id: action.id().to_owned(),
        interaction_id: context.interaction_id,
        trigger: context.trigger,
        event_id: context
            .notification
            .as_ref()
            .map(|notification| notification.event_id.clone()),
        started_at_ms,
        finished_at_ms: unix_time_ms(),
        outcome,
        exit_code,
        stdout,
        stderr,
    }
}

async fn capture_output<R>(reader: Option<R>) -> io::Result<CapturedOutput>
where
    R: AsyncRead + Unpin,
{
    let Some(mut reader) = reader else {
        return Err(io::Error::other("child output pipe is unavailable"));
    };
    let mut captured = Vec::with_capacity(MAX_ACTION_OUTPUT_BYTES);
    let mut total_bytes = 0_u64;
    let mut chunk = [0_u8; 4096];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        total_bytes = total_bytes.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        let remaining = MAX_ACTION_OUTPUT_BYTES.saturating_sub(captured.len());
        captured.extend_from_slice(&chunk[..read.min(remaining)]);
    }
    Ok(CapturedOutput {
        truncated_bytes: total_bytes.saturating_sub(captured.len() as u64),
        bytes: captured,
    })
}

async fn join_capture(mut task: JoinHandle<io::Result<CapturedOutput>>) -> Option<CapturedOutput> {
    match tokio::time::timeout(PROCESS_IO_DRAIN_TIMEOUT, &mut task).await {
        Ok(result) => result.ok()?.ok(),
        Err(_) => {
            task.abort();
            None
        }
    }
}

async fn join_stdin(task: Option<JoinHandle<io::Result<()>>>) -> Option<()> {
    let mut task = task?;
    match tokio::time::timeout(PROCESS_IO_DRAIN_TIMEOUT, &mut task).await {
        Ok(result) => result.ok()?.ok(),
        Err(_) => {
            task.abort();
            None
        }
    }
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(all(test, unix))]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::{
        ActionLoadContext, InteractionTrigger, PetLifecycleSnapshotV1, PetSnapshotV1,
        load_actions_str,
    };

    fn interaction() -> InteractionContextV1 {
        InteractionContextV1::for_pet(
            Uuid::new_v4(),
            1,
            InteractionTrigger::PetClick,
            PetSnapshotV1 {
                pet_id: "lili".to_owned(),
                label: "Lili".to_owned(),
                lifecycle: PetLifecycleSnapshotV1::Idle,
            },
        )
        .unwrap()
    }

    fn supervisor(command: &[&str], policy: &str, debounce_ms: u64) -> ActionSupervisor {
        let command = command
            .iter()
            .map(|part| format!("{part:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!(
            r#"
version = 1

[[action]]
id = "test"
trigger = "pet_click"
command = [{command}]
timeout_ms = 100
debounce_ms = {debounce_ms}

[action.concurrency]
{policy}
"#,
        );
        let loaded = load_actions_str(&source, &ActionLoadContext::new("/", "/", Vec::new()));
        assert!(loaded.effective().diagnostics.is_empty());
        ActionSupervisor::new(loaded, 1).unwrap()
    }

    fn two_action_supervisor() -> ActionSupervisor {
        let source = r#"
version = 1

[[action]]
id = "first"
trigger = "pet_click"
command = ["/bin/sh", "-c", "sleep 0.06"]
timeout_ms = 100
debounce_ms = 0

[[action]]
id = "second"
trigger = "pet_click"
command = ["/bin/sh", "-c", "sleep 0.06"]
timeout_ms = 100
debounce_ms = 0
"#;
        let loaded = load_actions_str(source, &ActionLoadContext::new("/", "/", Vec::new()));
        assert!(loaded.effective().diagnostics.is_empty());
        ActionSupervisor::new(loaded, 1).unwrap()
    }

    #[tokio::test]
    async fn debounce_reject_and_timeout_are_deterministic() {
        let debounced = supervisor(
            &["/bin/sh", "-c", "exit 0"],
            "mode = \"reject\"\nmax_parallel = 1\nqueue_capacity = 0",
            1000,
        );
        assert_eq!(
            debounced.execute("test", &interaction()).await.outcome,
            ActionExecutionOutcome::Succeeded
        );
        assert_eq!(
            debounced.execute("test", &interaction()).await.outcome,
            ActionExecutionOutcome::Debounced
        );

        let timed_out = supervisor(
            &["/bin/sh", "-c", "sleep 1"],
            "mode = \"reject\"\nmax_parallel = 1\nqueue_capacity = 0",
            0,
        );
        assert_eq!(
            timed_out.execute("test", &interaction()).await.outcome,
            ActionExecutionOutcome::TimedOut
        );
    }

    #[tokio::test]
    async fn output_is_bounded_and_overflow_is_a_failure() {
        let supervisor = supervisor(
            &["/bin/sh", "-c", "head -c 20000 /dev/zero"],
            "mode = \"reject\"\nmax_parallel = 1\nqueue_capacity = 0",
            0,
        );
        let result = supervisor.execute("test", &interaction()).await;
        assert_eq!(result.outcome, ActionExecutionOutcome::OutputOverflow);
        assert_eq!(result.stdout.bytes().len(), MAX_ACTION_OUTPUT_BYTES);
        assert_eq!(result.stdout.truncated_bytes(), 20000 - 16384);
    }

    #[tokio::test]
    async fn reject_mode_refuses_an_action_that_is_already_running() {
        let supervisor = supervisor(
            &["/bin/sh", "-c", "sleep 0.08"],
            "mode = \"reject\"\nmax_parallel = 1\nqueue_capacity = 0",
            0,
        );
        let running_supervisor = supervisor.clone();
        let running =
            tokio::spawn(async move { running_supervisor.execute("test", &interaction()).await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(
            supervisor.execute("test", &interaction()).await.outcome,
            ActionExecutionOutcome::Saturated
        );
        assert_eq!(
            running.await.unwrap().outcome,
            ActionExecutionOutcome::Succeeded
        );
    }

    #[tokio::test]
    async fn queue_mode_bounds_waiters_and_runs_the_admitted_entry() {
        let supervisor = supervisor(
            &["/bin/sh", "-c", "sleep 0.06"],
            "mode = \"queue\"\nmax_parallel = 1\nqueue_capacity = 1",
            0,
        );
        let first_supervisor = supervisor.clone();
        let first =
            tokio::spawn(async move { first_supervisor.execute("test", &interaction()).await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        let second_supervisor = supervisor.clone();
        let second =
            tokio::spawn(async move { second_supervisor.execute("test", &interaction()).await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(
            supervisor.execute("test", &interaction()).await.outcome,
            ActionExecutionOutcome::Saturated
        );
        assert_eq!(
            first.await.unwrap().outcome,
            ActionExecutionOutcome::Succeeded
        );
        assert_eq!(
            second.await.unwrap().outcome,
            ActionExecutionOutcome::Succeeded
        );
    }

    #[tokio::test]
    async fn global_limit_serializes_distinct_actions() {
        let supervisor = two_action_supervisor();
        let started = Instant::now();
        let first_context = interaction();
        let second_context = interaction();
        let (first, second) = tokio::join!(
            supervisor.execute("first", &first_context),
            supervisor.execute("second", &second_context),
        );
        assert_eq!(first.outcome, ActionExecutionOutcome::Succeeded);
        assert_eq!(second.outcome, ActionExecutionOutcome::Succeeded);
        assert!(started.elapsed() >= Duration::from_millis(100));
    }

    #[tokio::test]
    async fn audit_is_bounded_and_excludes_captured_output() {
        let supervisor = supervisor(
            &["/bin/sh", "-c", "printf private-output"],
            "mode = \"reject\"\nmax_parallel = 1\nqueue_capacity = 0",
            0,
        );
        let result = supervisor.execute("test", &interaction()).await;
        assert_eq!(result.stdout.bytes(), b"private-output");
        for _ in 0..=MAX_ACTION_AUDIT_ENTRIES {
            supervisor.record_audit(&result).await;
        }
        let audit = supervisor.audit_snapshot().await;
        assert_eq!(audit.len(), MAX_ACTION_AUDIT_ENTRIES);
        assert!(audit.iter().all(|entry| entry.stdout_captured_bytes == 14));
        let serialized = serde_json::to_string(&audit).unwrap();
        assert!(!serialized.contains("private-output"));
        assert!(!serialized.contains("environment"));
    }
}
