use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use tokio::{sync::watch, task::JoinHandle, time::Instant};

use crate::{ActionExecutionOutcome, ActionSupervisor, SessionTitleRequest, decode_title_response};

const CAPACITY: usize = 256;
type Key = (String, String, String);
type Reply = Option<Option<String>>;

#[derive(Default)]
pub(crate) struct TitleState {
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
        let action_id = self.title_action_id(provider)?.to_owned();
        let runtime = self.actions.get(&action_id)?.clone();
        let key = (action_id, provider.to_owned(), session_id.to_owned());
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
                let work = async {
                    let _action = runtime.running.clone().acquire_owned().await.ok()?;
                    let _global = supervisor.global.clone().acquire_owned().await.ok()?;
                    let mut result = crate::supervisor::run_action(&runtime.action, &request).await;
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
                    supervisor.record_audit(&result).await;
                    title
                };
                let title = if *shutdown.borrow() {
                    None
                } else {
                    tokio::select! {
                        biased;
                        _ = shutdown.changed() => None,
                        title = work => title,
                    }
                };
                let mut state = supervisor.titles.lock().await;
                if !*supervisor.shutdown.borrow() {
                    if let Some(title) = &title {
                        state.insert(worker_key.clone(), title.clone());
                    }
                }
                sender.send_replace(Some(title));
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
        loop {
            if let Some(reply) = receiver.borrow().clone() {
                return reply;
            }
            if receiver.changed().await.is_err() {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn key(index: usize) -> Key {
        ("title".into(), "example".into(), index.to_string())
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
