use std::collections::HashMap;

use lili_actions::ActionSupervisor;
use lili_session::{EventId, Notification, NotificationId, NotificationState, ProviderId};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::{AppState, AppStateStore, PersistenceError, PersistentApplicationState};

const MAX_PENDING_TITLES: usize = 256;

#[derive(Default)]
pub(super) struct TitleDispatch {
    pub pending: HashMap<NotificationId, PendingTitle>,
}

pub(super) struct PendingTitle {
    token: Uuid,
    pub worker: JoinHandle<()>,
}

impl AppState {
    pub async fn shutdown_title_actions(&self) {
        let runtime = self.action_runtime.read().await;
        let pending = std::mem::take(&mut self.title_dispatch.lock().await.pending);
        for task in pending.values() {
            task.worker.abort();
        }
        for task in pending.into_values() {
            let _ = task.worker.await;
        }
        if let Some(supervisor) = &runtime.supervisor {
            supervisor.shutdown_titles().await;
        }
    }

    pub(super) async fn schedule_notification_titles(
        &self,
        event: Option<(ProviderId, EventId)>,
        store: Option<AppStateStore>,
    ) {
        let runtime = self.action_runtime.read().await;
        let Some(supervisor) = runtime.supervisor.clone() else {
            return;
        };
        let notifications = self.session_reducer.lock().await.snapshot().notifications;
        let unread: std::collections::HashSet<_> = notifications
            .iter()
            .filter(|notification| notification.state == NotificationState::Unread)
            .map(|notification| notification.id.clone())
            .collect();
        let mut dispatch = self.title_dispatch.lock().await;
        dispatch.pending.retain(|id, task| {
            if unread.contains(id) {
                true
            } else {
                task.worker.abort();
                false
            }
        });
        for notification in notifications {
            if notification.state != NotificationState::Unread
                || event.as_ref().is_some_and(|(provider, id)| {
                    &notification.provider != provider || &notification.event_id != id
                })
                || supervisor
                    .title_action_id(notification.provider.as_str())
                    .is_none()
                || dispatch.pending.contains_key(&notification.id)
                || dispatch.pending.len() >= MAX_PENDING_TITLES
            {
                continue;
            }
            let token = Uuid::new_v4();
            let id = notification.id.clone();
            let state = self.clone();
            let supervisor = supervisor.clone();
            let store = store.clone();
            let worker = tokio::spawn(async move {
                let title = supervisor
                    .resolve_title(
                        notification.provider.as_str(),
                        notification.session_id.as_str(),
                    )
                    .await;
                state
                    .complete_title(supervisor, notification, token, title, store)
                    .await;
            });
            dispatch.pending.insert(id, PendingTitle { token, worker });
        }
    }

    async fn complete_title(
        &self,
        supervisor: ActionSupervisor,
        notification: Notification,
        token: Uuid,
        title: Option<String>,
        store: Option<AppStateStore>,
    ) {
        let runtime = self.action_runtime.read().await;
        if !runtime
            .supervisor
            .as_ref()
            .is_some_and(|current| current.same_configuration(&supervisor))
        {
            return;
        }
        let mut dispatch = self.title_dispatch.lock().await;
        if !dispatch
            .pending
            .get(&notification.id)
            .is_some_and(|task| task.token == token)
        {
            return;
        }
        dispatch.pending.remove(&notification.id);
        let Some(title) = title else {
            return;
        };
        let result = self
            .save_notification_title(&notification, title, store.as_ref())
            .await;
        drop(dispatch);
        drop(runtime);
        match result {
            Ok(true) => self.publish_presentation().await,
            Ok(false) => {}
            Err(_) => tracing::warn!(
                trigger = "session_title",
                request_id = %token,
                failure_kind = "persistence_failed",
                "notification title could not be saved"
            ),
        }
    }

    async fn save_notification_title(
        &self,
        notification: &Notification,
        title: String,
        store: Option<&AppStateStore>,
    ) -> Result<bool, PersistenceError> {
        let selected_pet_id =
            lili_core::PetId::parse(self.pet_state.read().await.catalog.requested_identifier());
        let mut reducer = self.session_reducer.lock().await;
        let previous = store.map(|_| reducer.clone());
        if !reducer.update_notification_title(notification, title) {
            return Ok(false);
        }
        if let Some(store) = store {
            let persistent =
                PersistentApplicationState::new(selected_pet_id, None, reducer.persistent_state());
            if let Err(error) = store.save(&persistent) {
                *reducer = previous.expect("persistent update retains rollback state");
                return Err(error);
            }
        }
        Ok(true)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use lili_actions::{ActionLoadContext, load_actions_str};
    use lili_session::{ProviderInputV1, normalize_provider_input};

    fn event(id: &str, session: &str) -> lili_session::NormalizedSessionEvent {
        normalize_provider_input(
            serde_json::from_value::<ProviderInputV1>(serde_json::json!({
                "version": 1, "provider": "example", "type": "turn_completed",
                "eventId": id, "sessionId": session, "turnId": id,
                "occurredAtMs": 10, "summary": "Keep summary"
            }))
            .unwrap(),
        )
        .unwrap()
    }

    async fn configure(state: &AppState, title: &str) {
        let script = format!(
            "cat >/dev/null; sleep 0.03; printf '%s' '{}'",
            serde_json::json!({"version":1,"title":title})
        );
        let source = format!(
            "version = 1\n[[action]]\nid = \"title\"\ntrigger = \"session_title\"\ncommand = [\"/bin/sh\", \"-c\", {}]\ndebounce_ms = 0\n",
            toml_string(&script)
        );
        let loaded = load_actions_str(&source, &ActionLoadContext::new("/", "/", vec![]));
        assert!(state.configure_actions(loaded, 1).await);
    }

    fn toml_string(value: &str) -> String {
        serde_json::to_string(value).unwrap()
    }

    async fn settle(state: &AppState) {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if state.title_dispatch.lock().await.pending.is_empty() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn updates_bound_notification_and_persists_independently_of_cache() {
        let state = AppState::default();
        configure(&state, "Custom title").await;
        let root = std::env::temp_dir().join(format!("title-store-{}", Uuid::new_v4()));
        let store = AppStateStore::for_application(
            lili_storage::ApplicationPaths::from_root(root.clone()).unwrap(),
        );
        state
            .apply_session_event_persisted(event("first", "one"), &store)
            .await
            .unwrap();
        assert!(
            state.pet_presentation().await.notifications[0]
                .title
                .is_none()
        );
        settle(&state).await;
        let presentation = state.pet_presentation().await;
        assert_eq!(
            presentation.notifications[0].title.as_deref(),
            Some("Custom title")
        );
        assert_eq!(presentation.notifications[0].summary, "Keep summary");
        assert!(presentation.action_feedback.is_none());
        state.shutdown_title_actions().await;
        assert_eq!(
            state.pet_presentation().await.notifications[0].title,
            presentation.notifications[0].title
        );
        let restored = AppState::with_persistent_state(
            lili_pet::PetCatalog::default(),
            store.load().unwrap().unwrap(),
        )
        .unwrap();
        assert_eq!(
            restored.pet_presentation().await.notifications[0]
                .title
                .as_deref(),
            Some("Custom title")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn dismissal_and_configuration_changes_discard_old_results() {
        let state = AppState::default();
        configure(&state, "Old").await;
        state.apply_session_event(event("first", "one")).await;
        let id = state.snapshot().await.session_state.notifications[0]
            .id
            .clone();
        state.acknowledge_notification(&id).await;
        settle(&state).await;
        assert!(state.pet_presentation().await.notifications.is_empty());
        state.apply_session_event(event("second", "two")).await;
        configure(&state, "New").await;
        settle(&state).await;
        assert_eq!(
            state.pet_presentation().await.notifications[0]
                .title
                .as_deref(),
            Some("New")
        );
    }

    #[tokio::test]
    async fn old_notification_payloads_load_and_failed_title_save_rolls_back() {
        let state = AppState::default();
        state.apply_session_event(event("first", "one")).await;
        let notification = state.snapshot().await.session_state.notifications[0].clone();
        let mut json = serde_json::to_value(&notification).unwrap();
        json.as_object_mut().unwrap().remove("title");
        let old: Notification = serde_json::from_value(json.clone()).unwrap();
        assert!(old.title.is_none());
        json["title"] = serde_json::json!("x".repeat(257));
        assert!(serde_json::from_value::<Notification>(json).is_err());
        let root = std::env::temp_dir().join(format!("title-invalid-store-{}", Uuid::new_v4()));
        std::fs::write(&root, b"not a directory").unwrap();
        let store = AppStateStore::for_application(
            lili_storage::ApplicationPaths::from_root(root.clone()).unwrap(),
        );
        assert!(
            state
                .save_notification_title(&notification, "New title".into(), Some(&store))
                .await
                .is_err()
        );
        assert!(
            state.snapshot().await.session_state.notifications[0]
                .title
                .is_none()
        );
        std::fs::remove_file(root).unwrap();
    }

    #[tokio::test]
    async fn other_sessions_do_not_receive_a_bound_result() {
        let state = AppState::default();
        configure(&state, "First title").await;
        state.apply_session_event(event("first", "one")).await;
        // Reject mode admits only the first execution while the second is busy.
        state.apply_session_event(event("second", "two")).await;
        settle(&state).await;
        let snapshot = state.snapshot().await;
        assert_eq!(
            snapshot
                .session_state
                .notifications
                .iter()
                .find(|n| n.session_id.as_str() == "one")
                .unwrap()
                .title
                .as_deref(),
            Some("First title")
        );
        assert!(
            snapshot
                .session_state
                .notifications
                .iter()
                .find(|n| n.session_id.as_str() == "two")
                .unwrap()
                .title
                .is_none()
        );
    }
}
