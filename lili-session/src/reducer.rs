use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt::Write as _,
};

use sha2::{Digest, Sha256};

use crate::{
    DisplayProjectContext, EventId, NormalizedSessionEvent, Notification, NotificationId,
    NotificationKind, NotificationState, PresentationState, ProviderId, SessionEventKind,
    SessionId, SessionPhase, SessionSummary, SessionViewSnapshot, TurnId,
};

const MAX_RECENT_EVENT_IDS: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReductionOutcome {
    Applied { revision: u64 },
    Duplicate,
    IgnoredStale,
}

#[derive(Clone, Debug, Default)]
pub struct SessionReducer {
    revision: u64,
    sessions: BTreeMap<(ProviderId, SessionId), SessionRecord>,
    notifications: BTreeMap<NotificationId, Notification>,
    recent_event_ids: VecDeque<(ProviderId, EventId)>,
    recent_event_set: BTreeSet<(ProviderId, EventId)>,
}

#[derive(Clone, Debug)]
struct SessionRecord {
    provider: ProviderId,
    id: SessionId,
    current_turn_id: Option<TurnId>,
    turns: BTreeMap<TurnId, TurnRecord>,
    project: Option<DisplayProjectContext>,
    updated_at_ms: u64,
    ended: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TurnRecord {
    phase: SessionPhase,
    updated_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Transition {
    changed: bool,
    notification: Option<NotificationKind>,
    resolve_attention: bool,
}

impl SessionReducer {
    pub fn reduce(&mut self, event: NormalizedSessionEvent) -> ReductionOutcome {
        let event_key = (event.provider.clone(), event.event_id.clone());
        if self.recent_event_set.contains(&event_key) {
            return ReductionOutcome::Duplicate;
        }
        self.remember_event(event_key);

        let transition = match event.event_type {
            SessionEventKind::SessionStarted | SessionEventKind::SessionEnded => {
                self.apply_session_event(&event)
            }
            _ => self.apply_turn_event(&event),
        };
        if !transition.changed {
            return ReductionOutcome::IgnoredStale;
        }

        if transition.resolve_attention {
            self.resolve_attention_notifications(
                &event.provider,
                &event.session_id,
                event.turn_id.as_ref(),
            );
        }
        if let Some(kind) = transition.notification {
            self.insert_notification(&event, kind);
        }
        self.revision = self.revision.saturating_add(1);
        ReductionOutcome::Applied {
            revision: self.revision,
        }
    }

    pub fn acknowledge_notification(&mut self, id: &NotificationId) -> ReductionOutcome {
        let Some(notification) = self.notifications.get_mut(id) else {
            return ReductionOutcome::IgnoredStale;
        };
        if notification.state != NotificationState::Unread {
            return ReductionOutcome::IgnoredStale;
        }
        notification.state = NotificationState::Acknowledged;
        self.revision = self.revision.saturating_add(1);
        ReductionOutcome::Applied {
            revision: self.revision,
        }
    }

    pub fn snapshot(&self) -> SessionViewSnapshot {
        let mut sessions = self
            .sessions
            .values()
            .map(SessionRecord::summary)
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.provider.cmp(&right.provider))
                .then_with(|| left.id.cmp(&right.id))
        });
        let mut notifications = self.notifications.values().cloned().collect::<Vec<_>>();
        notifications.sort_by(|left, right| {
            right
                .occurred_at_ms
                .cmp(&left.occurred_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        SessionViewSnapshot {
            revision: self.revision,
            presentation: PresentationState::Idle,
            sessions,
            notifications,
        }
    }

    fn apply_session_event(&mut self, event: &NormalizedSessionEvent) -> Transition {
        let key = (event.provider.clone(), event.session_id.clone());
        match event.event_type {
            SessionEventKind::SessionStarted => {
                if let Some(session) = self.sessions.get_mut(&key) {
                    if event.occurred_at_ms < session.updated_at_ms || !session.ended {
                        return Transition::default();
                    }
                    session.ended = false;
                    session.current_turn_id = None;
                    session.updated_at_ms = event.occurred_at_ms;
                    if event.project.is_some() {
                        session.project.clone_from(&event.project);
                    }
                } else {
                    self.sessions.insert(key, SessionRecord::new(event));
                }
                Transition {
                    changed: true,
                    ..Transition::default()
                }
            }
            SessionEventKind::SessionEnded => {
                let Some(session) = self.sessions.get_mut(&key) else {
                    return Transition::default();
                };
                if event.occurred_at_ms < session.updated_at_ms || session.ended {
                    return Transition::default();
                }
                session.ended = true;
                session.updated_at_ms = event.occurred_at_ms;
                Transition {
                    changed: true,
                    resolve_attention: true,
                    ..Transition::default()
                }
            }
            _ => unreachable!("turn events use apply_turn_event"),
        }
    }

    fn apply_turn_event(&mut self, event: &NormalizedSessionEvent) -> Transition {
        let turn_id = event
            .turn_id
            .as_ref()
            .expect("normalized turn event must have a turn identity")
            .clone();
        let key = (event.provider.clone(), event.session_id.clone());

        if event.event_type == SessionEventKind::AttentionResolved {
            let Some(session) = self.sessions.get_mut(&key) else {
                return Transition::default();
            };
            let Some(turn) = session.turns.get_mut(&turn_id) else {
                return Transition::default();
            };
            if event.occurred_at_ms < turn.updated_at_ms || turn.phase != SessionPhase::Attention {
                return Transition::default();
            }
            turn.phase = SessionPhase::Active;
            turn.updated_at_ms = event.occurred_at_ms;
            update_session_after_turn(session, event, &turn_id);
            return Transition {
                changed: true,
                resolve_attention: true,
                ..Transition::default()
            };
        }

        let session = self
            .sessions
            .entry(key)
            .or_insert_with(|| SessionRecord::new(event));
        let previous = session.turns.get(&turn_id).copied();
        let Some(next_phase) = next_turn_phase(previous, event) else {
            return Transition::default();
        };
        session.turns.insert(
            turn_id.clone(),
            TurnRecord {
                phase: next_phase,
                updated_at_ms: event.occurred_at_ms,
            },
        );
        session.ended = false;
        update_session_after_turn(session, event, &turn_id);

        Transition {
            changed: true,
            notification: match event.event_type {
                SessionEventKind::AttentionRequired => Some(NotificationKind::Attention),
                SessionEventKind::TurnCompleted => Some(NotificationKind::Completion),
                SessionEventKind::TurnFailed => Some(NotificationKind::Failure),
                _ => None,
            },
            resolve_attention: matches!(
                event.event_type,
                SessionEventKind::TurnCompleted | SessionEventKind::TurnFailed
            ),
        }
    }

    fn insert_notification(&mut self, event: &NormalizedSessionEvent, kind: NotificationKind) {
        let id = notification_id(&event.provider, &event.event_id);
        self.notifications
            .entry(id.clone())
            .or_insert_with(|| Notification {
                id,
                provider: event.provider.clone(),
                event_id: event.event_id.clone(),
                session_id: event.session_id.clone(),
                turn_id: event.turn_id.clone(),
                kind,
                state: NotificationState::Unread,
                occurred_at_ms: event.occurred_at_ms,
                project: event.project.clone(),
                summary: event.summary.clone(),
            });
    }

    fn resolve_attention_notifications(
        &mut self,
        provider: &ProviderId,
        session_id: &SessionId,
        turn_id: Option<&TurnId>,
    ) {
        for notification in self.notifications.values_mut() {
            if notification.provider == *provider
                && notification.session_id == *session_id
                && turn_id.is_none_or(|turn_id| notification.turn_id.as_ref() == Some(turn_id))
                && notification.kind == NotificationKind::Attention
            {
                notification.state = NotificationState::Resolved;
            }
        }
    }

    fn remember_event(&mut self, event: (ProviderId, EventId)) {
        self.recent_event_set.insert(event.clone());
        self.recent_event_ids.push_back(event);
        while self.recent_event_ids.len() > MAX_RECENT_EVENT_IDS {
            if let Some(expired) = self.recent_event_ids.pop_front() {
                self.recent_event_set.remove(&expired);
            }
        }
    }
}

impl SessionRecord {
    fn new(event: &NormalizedSessionEvent) -> Self {
        Self {
            provider: event.provider.clone(),
            id: event.session_id.clone(),
            current_turn_id: None,
            turns: BTreeMap::new(),
            project: event.project.clone(),
            updated_at_ms: event.occurred_at_ms,
            ended: false,
        }
    }

    fn summary(&self) -> SessionSummary {
        let phase = if self.ended {
            SessionPhase::Ended
        } else {
            self.current_turn_id
                .as_ref()
                .and_then(|turn_id| self.turns.get(turn_id))
                .map_or(SessionPhase::Idle, |turn| turn.phase)
        };
        SessionSummary {
            provider: self.provider.clone(),
            id: self.id.clone(),
            current_turn_id: self.current_turn_id.clone(),
            phase,
            project: self.project.clone(),
            updated_at_ms: self.updated_at_ms,
        }
    }
}

fn next_turn_phase(
    previous: Option<TurnRecord>,
    event: &NormalizedSessionEvent,
) -> Option<SessionPhase> {
    if let Some(previous) = previous {
        if event.occurred_at_ms < previous.updated_at_ms
            || matches!(
                previous.phase,
                SessionPhase::Completed | SessionPhase::Failed
            )
        {
            return None;
        }
        if previous.phase == SessionPhase::Attention
            && event.event_type == SessionEventKind::TurnStarted
        {
            return None;
        }
    }
    match event.event_type {
        SessionEventKind::TurnStarted => Some(SessionPhase::Active),
        SessionEventKind::AttentionRequired => Some(SessionPhase::Attention),
        SessionEventKind::TurnCompleted => Some(SessionPhase::Completed),
        SessionEventKind::TurnFailed => Some(SessionPhase::Failed),
        _ => None,
    }
}

fn update_session_after_turn(
    session: &mut SessionRecord,
    event: &NormalizedSessionEvent,
    turn_id: &TurnId,
) {
    let current_order = session.current_turn_id.as_ref().and_then(|current_id| {
        session
            .turns
            .get(current_id)
            .map(|turn| (turn.updated_at_ms, current_id))
    });
    let incoming_order = (event.occurred_at_ms, turn_id);
    if current_order.is_none_or(|current| incoming_order >= current) {
        session.current_turn_id = Some(turn_id.clone());
    }
    if event.occurred_at_ms >= session.updated_at_ms {
        session.updated_at_ms = event.occurred_at_ms;
        if event.project.is_some() {
            session.project.clone_from(&event.project);
        }
    }
}

fn notification_id(provider: &ProviderId, event_id: &EventId) -> NotificationId {
    let mut digest = Sha256::new();
    for field in [provider.as_str(), event_id.as_str()] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field.as_bytes());
    }
    let mut identity = String::from("notification-");
    for byte in digest.finalize() {
        write!(&mut identity, "{byte:02x}").expect("writing to a string cannot fail");
    }
    NotificationId::parse(identity).expect("notification identity is bounded")
}

#[cfg(test)]
mod tests {
    use crate::{ProviderCapabilitiesInputV1, ProviderInputV1, normalize_provider_input};

    use super::*;

    fn event(
        event_id: &str,
        event_type: &str,
        session_id: &str,
        turn_id: Option<&str>,
        occurred_at_ms: u64,
    ) -> NormalizedSessionEvent {
        normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some(event_type.to_owned()),
            event_id: Some(event_id.to_owned()),
            session_id: Some(session_id.to_owned()),
            turn_id: turn_id.map(str::to_owned),
            occurred_at_ms: Some(occurred_at_ms),
            project: None,
            summary: None,
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap()
    }

    #[test]
    fn duplicate_event_advances_revision_once() {
        let mut reducer = SessionReducer::default();
        let event = event("event-1", "turn_started", "session-1", Some("turn-1"), 1);
        assert_eq!(
            reducer.reduce(event.clone()),
            ReductionOutcome::Applied { revision: 1 }
        );
        assert_eq!(reducer.reduce(event), ReductionOutcome::Duplicate);
        assert_eq!(reducer.snapshot().revision, 1);
    }

    #[test]
    fn terminal_turn_cannot_return_to_active_but_new_turn_can() {
        let mut reducer = SessionReducer::default();
        reducer.reduce(event(
            "event-1",
            "turn_completed",
            "session-1",
            Some("turn-1"),
            10,
        ));
        assert_eq!(
            reducer.reduce(event(
                "event-2",
                "turn_started",
                "session-1",
                Some("turn-1"),
                11,
            )),
            ReductionOutcome::IgnoredStale
        );
        assert_eq!(
            reducer.snapshot().sessions[0].phase,
            SessionPhase::Completed
        );

        assert_eq!(
            reducer.reduce(event(
                "event-3",
                "turn_started",
                "session-1",
                Some("turn-2"),
                12,
            )),
            ReductionOutcome::Applied { revision: 2 }
        );
        assert_eq!(reducer.snapshot().sessions[0].phase, SessionPhase::Active);
    }

    #[test]
    fn attention_resolution_updates_state_and_notification() {
        let mut reducer = SessionReducer::default();
        reducer.reduce(event(
            "event-1",
            "attention_required",
            "session-1",
            Some("turn-1"),
            10,
        ));
        let notification_id = reducer.snapshot().notifications[0].id.clone();
        assert_eq!(
            reducer.reduce(event(
                "event-2",
                "attention_resolved",
                "session-1",
                Some("turn-1"),
                11,
            )),
            ReductionOutcome::Applied { revision: 2 }
        );
        let snapshot = reducer.snapshot();
        assert_eq!(snapshot.sessions[0].phase, SessionPhase::Active);
        assert_eq!(snapshot.notifications[0].state, NotificationState::Resolved);
        assert_eq!(
            reducer.acknowledge_notification(&notification_id),
            ReductionOutcome::IgnoredStale
        );
    }

    #[test]
    fn unread_notification_is_acknowledged_once() {
        let mut reducer = SessionReducer::default();
        reducer.reduce(event(
            "event-1",
            "turn_failed",
            "session-1",
            Some("turn-1"),
            10,
        ));
        let id = reducer.snapshot().notifications[0].id.clone();
        assert_eq!(
            reducer.acknowledge_notification(&id),
            ReductionOutcome::Applied { revision: 2 }
        );
        assert_eq!(
            reducer.acknowledge_notification(&id),
            ReductionOutcome::IgnoredStale
        );
        assert_eq!(
            reducer.snapshot().notifications[0].state,
            NotificationState::Acknowledged
        );
    }

    #[test]
    fn session_end_resolves_attention_for_every_turn() {
        let mut reducer = SessionReducer::default();
        for (index, turn_id) in ["turn-1", "turn-2"].into_iter().enumerate() {
            reducer.reduce(event(
                &format!("event-{index}"),
                "attention_required",
                "session-1",
                Some(turn_id),
                index as u64 + 1,
            ));
        }
        reducer.reduce(event("event-end", "session_ended", "session-1", None, 10));
        let snapshot = reducer.snapshot();
        assert_eq!(snapshot.sessions[0].phase, SessionPhase::Ended);
        assert!(
            snapshot
                .notifications
                .iter()
                .all(|notification| notification.state == NotificationState::Resolved)
        );
    }
}
