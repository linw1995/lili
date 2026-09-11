use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use tokio::{sync::watch, task::JoinHandle, time::Instant};

use super::{
    ActionExecutionOutcome, ActionSupervisor, SessionTitleRequest, run_action_cancellable,
};
use crate::decode_title_response;

const CAPACITY: usize = 256;
// Provider selection is fixed within this immutable supervisor instance.
type Key = (String, String);
type Reply = Option<String>;

#[derive(Default)]
pub struct TitleState {
    cache: VecDeque<(Key, String)>,
    debounce: VecDeque<(Key, Instant)>,
    pending: BTreeMap<Key, Pending>,
}

struct Pending {
    receiver: watch::Receiver<Reply>,
    worker: JoinHandle<()>,
}

impl TitleState {
    fn cached(&mut self, key: &Key) -> Option<String> {
        let index = self
            .cache
            .iter()
            .position(|(candidate, _)| candidate == key)?;
        let entry = self.cache.remove(index)?;
        let title = entry.1.clone();
        self.cache.push_back(entry);
        Some(title)
    }

    fn insert(&mut self, key: Key, title: String) {
        self.cache.retain(|(candidate, _)| candidate != &key);
        if self.cache.len() == CAPACITY {
            self.cache.pop_front();
        }
        self.cache.push_back((key, title));
    }

    fn accept(&mut self, key: &Key, delay: u64) -> bool {
        let now = Instant::now();
        if let Some(index) = self
            .debounce
            .iter()
            .position(|(candidate, _)| candidate == key)
        {
            if now.duration_since(self.debounce[index].1) < Duration::from_millis(delay) {
                return false;
            }
            self.debounce.remove(index);
        }
        if self.debounce.len() == CAPACITY {
            self.debounce.pop_front();
        }
        self.debounce.push_back((key.clone(), now));
        true
    }
}

impl ActionSupervisor {
    pub fn title_action_id(&self, provider: &str) -> Option<&str> {
        self.title_order
            .iter()
            .find(|id| {
                let filters = self.actions[*id].action.filters();
                filters.providers.is_empty()
                    || filters.providers.iter().any(|value| value == provider)
            })
            .map(String::as_str)
    }

    pub fn same_configuration(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.actions, &other.actions)
    }

    pub async fn shutdown_titles(&self) {
        self.shutdown.send_replace(true);
        let pending = {
            let mut state = self.titles.lock().await;
            state.cache.clear();
            state.debounce.clear();
            std::mem::take(&mut state.pending)
        };
        for pending in pending.into_values() {
            let _ = pending.worker.await;
        }
    }

    pub async fn resolve_title(&self, provider: &str, session_id: &str) -> Option<String> {
        if *self.shutdown.borrow() {
            return None;
        }
        let action_id = self.title_action_id(provider)?;
        let runtime = self.actions.get(action_id)?.clone();
        let key = (provider.to_owned(), session_id.to_owned());
        let mut state = self.titles.lock().await;
        if let Some(title) = state.cached(&key) {
            return Some(title);
        }
        let mut receiver = if let Some(pending) = state.pending.get(&key) {
            pending.receiver.clone()
        } else {
            if state.pending.len() >= CAPACITY {
                return None;
            }
            let admission = runtime.admit()?;
            if !state.accept(&key, runtime.action.debounce_ms()) {
                return None;
            }
            let (sender, receiver) = watch::channel(None);
            let supervisor = self.clone();
            let worker_key = key.clone();
            let request = SessionTitleRequest::new(provider.to_owned(), session_id.to_owned());
            let worker = tokio::spawn(async move {
                let _admission = admission;
                let mut shutdown = supervisor.shutdown.subscribe();
                let slots = async {
                    Some((
                        runtime.running.clone().acquire_owned().await.ok()?,
                        supervisor.global.clone().acquire_owned().await.ok()?,
                    ))
                };
                let slots = if *shutdown.borrow() {
                    None
                } else {
                    tokio::select! {
                        biased;
                        _ = shutdown.changed() => None,
                        slots = slots => slots,
                    }
                };
                let title = if let Some((_action, _global)) = slots {
                    let started = Instant::now();
                    if let Some(mut result) =
                        run_action_cancellable(&runtime.action, &request, Some(shutdown.clone()))
                            .await
                    {
                        let title = if result.outcome == ActionExecutionOutcome::Succeeded {
                            match decode_title_response(result.stdout.bytes()) {
                                Ok(title) => title,
                                Err(_) => {
                                    result.outcome = ActionExecutionOutcome::InvalidResponse;
                                    None
                                }
                            }
                        } else {
                            None
                        };
                        log_failure(&result, started.elapsed());
                        supervisor.record_audit(&result).await;
                        title
                    } else {
                        None
                    }
                } else {
                    None
                };
                let mut state = supervisor.titles.lock().await;
                if !*supervisor.shutdown.borrow()
                    && let Some(title) = &title
                {
                    state.insert(worker_key.clone(), title.clone());
                }
                sender.send_replace(title);
                state.pending.remove(&worker_key);
            });
            state.pending.insert(
                key,
                Pending {
                    receiver: receiver.clone(),
                    worker,
                },
            );
            receiver
        };
        drop(state);
        // Even a null result advances the watch version and wakes every waiter.
        receiver.changed().await.ok()?;
        receiver.borrow_and_update().clone()
    }
}

fn log_failure(result: &crate::ActionExecutionResult, duration: Duration) {
    let failure_kind = match result.outcome {
        ActionExecutionOutcome::SpawnFailed => "spawn_failed",
        ActionExecutionOutcome::IoFailed => "io_failed",
        ActionExecutionOutcome::TimedOut => "timed_out",
        ActionExecutionOutcome::NonZeroExit => "nonzero_exit",
        ActionExecutionOutcome::OutputOverflow => "output_overflow",
        ActionExecutionOutcome::InvalidResponse => "invalid_response",
        _ => return,
    };
    tracing::warn!(
        action_id = %result.action_id,
        request_id = %result.interaction_id,
        trigger = "session_title",
        failure_kind,
        duration_ms = duration.as_millis() as u64,
        exit_code = result.exit_code,
        "session title execution failed"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn coalesced_null_notifies_every_waiter() {
        let supervisor = supervisor(
            "cat >/dev/null; sleep 0.02; printf '%s' '{\"version\":1,\"title\":null}'",
            0,
        );
        let (a, b) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(
                supervisor.resolve_title("example", "one"),
                supervisor.resolve_title("example", "one")
            )
        })
        .await
        .unwrap();
        assert_eq!((a, b), (None, None));
        assert_eq!(supervisor.audit_snapshot().await.len(), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_results_obey_per_session_debounce() {
        let supervisor = supervisor("cat >/dev/null; printf invalid", 60000);
        assert!(supervisor.resolve_title("example", "one").await.is_none());
        assert!(supervisor.resolve_title("example", "one").await.is_none());
        assert_eq!(supervisor.audit_snapshot().await.len(), 1);
        assert!(supervisor.resolve_title("example", "two").await.is_none());
        assert_eq!(supervisor.audit_snapshot().await.len(), 2);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cache_keeps_providers_separate() {
        let mut source = String::from("version = 1\n");
        for (id, provider, title) in [
            ("z-first", "first", "First"),
            ("a-second", "second", "Second"),
        ] {
            let script = format!(
                "cat >/dev/null; printf '%s' '{}'",
                serde_json::json!({"version":1,"title":title})
            );
            source.push_str(&format!("\n[[action]]\nid = \"{id}\"\ntrigger = \"session_title\"\ncommand = [\"/bin/sh\", \"-c\", {}]\n[action.filters]\nproviders = [\"{provider}\"]\n", toml::Value::String(script)));
        }
        let loaded =
            crate::load_actions_str(&source, &crate::ActionLoadContext::new("/", "/", vec![]));
        let supervisor = ActionSupervisor::new(loaded, 1).unwrap();
        assert_eq!(
            supervisor.resolve_title("first", "shared").await.as_deref(),
            Some("First")
        );
        assert_eq!(
            supervisor
                .resolve_title("second", "shared")
                .await
                .as_deref(),
            Some("Second")
        );
        assert_eq!(
            supervisor.resolve_title("first", "shared").await.as_deref(),
            Some("First")
        );
        assert_eq!(supervisor.audit_snapshot().await.len(), 2);
    }

    #[cfg(unix)]
    fn supervisor(script: &str, debounce: u64) -> ActionSupervisor {
        let source = format!(
            "version = 1\n[[action]]\nid = \"z-first\"\ntrigger = \"session_title\"\ncommand = [\"/bin/sh\", \"-c\", {}]\ndebounce_ms = {debounce}\ntimeout_ms = 500\n[[action]]\nid = \"a-second\"\ntrigger = \"session_title\"\ncommand = [\"/bin/sh\", \"-c\", \"exit 1\"]\n",
            toml::Value::String(script.into())
        );
        let loaded =
            crate::load_actions_str(&source, &crate::ActionLoadContext::new("/", "/", vec![]));
        assert_eq!(loaded.enabled().len(), 2);
        ActionSupervisor::new(loaded, 1).unwrap()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn requests_coalesce_cache_and_select_in_file_order() {
        let supervisor = supervisor(
            "cat >/dev/null; sleep 0.02; printf '%s' '{\"version\":1,\"title\":\"custom\"}'",
            0,
        );
        assert_eq!(supervisor.title_action_id("example"), Some("z-first"));
        let (first, second) = tokio::join!(
            supervisor.resolve_title("example", "one"),
            supervisor.resolve_title("example", "one")
        );
        assert_eq!(first.as_deref(), Some("custom"));
        assert_eq!(first, second);
        assert_eq!(supervisor.audit_snapshot().await.len(), 1);
        assert_eq!(supervisor.resolve_title("example", "one").await, first);
        assert_eq!(supervisor.audit_snapshot().await.len(), 1);
        supervisor.shutdown_titles().await;
        assert!(supervisor.resolve_title("example", "one").await.is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn empty_and_invalid_results_are_not_cached() {
        for (output, outcome) in [
            (
                "{\"version\":1,\"title\":null}",
                ActionExecutionOutcome::Succeeded,
            ),
            ("invalid", ActionExecutionOutcome::InvalidResponse),
        ] {
            let supervisor = supervisor(&format!("cat >/dev/null; printf '%s' '{output}'"), 0);
            assert!(supervisor.resolve_title("example", "one").await.is_none());
            assert!(supervisor.resolve_title("example", "one").await.is_none());
            let audit = supervisor.audit_snapshot().await;
            assert_eq!(audit.len(), 2);
            assert!(audit.iter().all(|entry| entry.outcome == outcome));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_cancels_pending_queries() {
        let supervisor = supervisor("cat >/dev/null; sleep 30", 0);
        let worker = {
            let supervisor = supervisor.clone();
            tokio::spawn(async move { supervisor.resolve_title("example", "one").await })
        };
        while supervisor.titles.lock().await.pending.is_empty() {
            tokio::task::yield_now().await;
        }
        supervisor.shutdown_titles().await;
        assert_eq!(worker.await.unwrap(), None);
        assert!(supervisor.titles.lock().await.pending.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shutdown_waits_until_the_spawned_child_is_reaped() {
        let path = std::env::temp_dir().join(format!("title-child-{}", uuid::Uuid::new_v4()));
        let supervisor = supervisor(&format!("echo $$ > '{}'; exec sleep 30", path.display()), 0);
        let worker = {
            let supervisor = supervisor.clone();
            tokio::spawn(async move { supervisor.resolve_title("example", "one").await })
        };
        let pid = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(contents) = std::fs::read_to_string(&path)
                    && let Ok(pid) = contents.trim().parse::<libc::pid_t>()
                {
                    break pid;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
        supervisor.shutdown_titles().await;
        assert_eq!(worker.await.unwrap(), None);
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
        assert!(supervisor.audit_snapshot().await.is_empty());
        std::fs::remove_file(path).unwrap();
    }

    #[derive(Clone, Default)]
    struct LogBuffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for LogBuffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_coalesced_execution_logs_once_without_output() {
        let buffer = LogBuffer::default();
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .without_time()
            .with_writer(move || writer.clone())
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);
        let supervisor = supervisor(
            "cat >/dev/null; sleep 0.02; printf 'PRIVATE_OUTPUT'; printf 'PRIVATE_ERROR' >&2",
            0,
        );
        let _ = tokio::join!(
            supervisor.resolve_title("example", "one"),
            supervisor.resolve_title("example", "one")
        );
        let output = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        assert_eq!(output.lines().count(), 1);
        assert!(output.contains("invalid_response"));
        assert!(!output.contains("PRIVATE_OUTPUT"));
        assert!(!output.contains("PRIVATE_ERROR"));
    }

    #[test]
    fn warnings_have_bounded_metadata_and_only_failure_outcomes() {
        let buffer = LogBuffer::default();
        let writer = buffer.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .without_time()
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            for outcome in [
                ActionExecutionOutcome::SpawnFailed,
                ActionExecutionOutcome::IoFailed,
                ActionExecutionOutcome::TimedOut,
                ActionExecutionOutcome::NonZeroExit,
                ActionExecutionOutcome::OutputOverflow,
                ActionExecutionOutcome::InvalidResponse,
                ActionExecutionOutcome::Succeeded,
                ActionExecutionOutcome::Debounced,
                ActionExecutionOutcome::Saturated,
                ActionExecutionOutcome::NotMatched,
                ActionExecutionOutcome::UnknownAction,
            ] {
                log_failure(
                    &crate::ActionExecutionResult {
                        action_id: "title-action".into(),
                        interaction_id: uuid::Uuid::nil(),
                        trigger: crate::ActionTrigger::SessionTitle,
                        event_id: None,
                        started_at_ms: 0,
                        finished_at_ms: 7,
                        outcome,
                        exit_code: Some(9),
                        stdout: crate::CapturedOutput::default(),
                        stderr: crate::CapturedOutput::default(),
                    },
                    Duration::from_millis(7),
                );
            }
        });
        let output = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        assert_eq!(output.lines().count(), 6);
        for line in output.lines() {
            let event: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(event["level"], "WARN");
            let fields = &event["fields"];
            assert_eq!(fields["action_id"], "title-action");
            assert_eq!(fields["duration_ms"], 7);
            assert_eq!(fields["exit_code"], 9);
            assert_eq!(fields["request_id"], uuid::Uuid::nil().to_string());
            assert!(fields.get("failure_kind").is_some());
            for field in ["title", "stdout", "stderr", "request", "environment"] {
                assert!(fields.get(field).is_none());
            }
        }
    }

    fn key(index: usize) -> Key {
        ("example".into(), index.to_string())
    }

    #[test]
    fn lru_promotes_hits_and_evicts_only_least_recent() {
        let mut state = TitleState::default();
        for index in 0..CAPACITY {
            state.insert(key(index), format!("title-{index}"));
        }
        assert_eq!(state.cached(&key(0)).as_deref(), Some("title-0"));
        state.insert(key(CAPACITY), "new".into());
        assert!(state.cached(&key(1)).is_none());
        assert!(state.cached(&key(0)).is_some());
        assert_eq!(state.cache.len(), CAPACITY);
    }

    #[tokio::test(start_paused = true)]
    async fn debounce_is_per_key_and_bounded() {
        let mut state = TitleState::default();
        assert!(state.accept(&key(0), 250));
        assert!(!state.accept(&key(0), 250));
        assert!(state.accept(&key(1), 250));
        tokio::time::advance(Duration::from_millis(250)).await;
        assert!(state.accept(&key(0), 250));
        for index in 0..CAPACITY + 10 {
            state.accept(&key(index), 250);
        }
        assert_eq!(state.debounce.len(), CAPACITY);
    }
}
