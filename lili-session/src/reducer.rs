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
pub const DEFAULT_MINIMUM_DWELL_MS: u64 = 750;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReductionOutcome {
    Applied { revision: u64 },
    Duplicate,
    IgnoredStale,
}

#[derive(Clone, Debug)]
pub struct SessionReducer {
    revision: u64,
    sessions: BTreeMap<(ProviderId, SessionId), SessionRecord>,
    notifications: BTreeMap<NotificationId, Notification>,
    recent_event_ids: VecDeque<(ProviderId, EventId)>,
    recent_event_set: BTreeSet<(ProviderId, EventId)>,
    presentation: PresentationTracker,
    minimum_dwell_ms: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct PresentationTracker {
    state: PresentationState,
    since_ms: u64,
}

#[derive(Clone, Debug)]
struct SessionRecord {
    provider: ProviderId,
    id: SessionId,
    current_turn_id: Option<TurnId>,
    turns: BTreeMap<TurnId, TurnRecord>,
    project: Option<DisplayProjectContext>,
    updated_at_ms: u64,
    last_event_id: EventId,
    ended: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TurnRecord {
    phase: SessionPhase,
    updated_at_ms: u64,
    last_event_id: EventId,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Transition {
    changed: bool,
    notification: Option<NotificationKind>,
    resolve_attention: bool,
}

impl SessionReducer {
    pub fn with_minimum_dwell_ms(minimum_dwell_ms: u64) -> Self {
        Self {
            minimum_dwell_ms,
            ..Self::default()
        }
    }

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
            if matches!(
                kind,
                NotificationKind::Completion | NotificationKind::Failure
            ) {
                self.remove_superseded_terminal_notifications(&event);
            }
            self.insert_notification(&event, kind);
        }
        self.refresh_presentation(event.occurred_at_ms);
        self.revision = self.revision.saturating_add(1);
        ReductionOutcome::Applied {
            revision: self.revision,
        }
    }

    pub fn acknowledge_notification(
        &mut self,
        id: &NotificationId,
        now_ms: u64,
    ) -> ReductionOutcome {
        let Some(notification) = self.notifications.get_mut(id) else {
            return ReductionOutcome::IgnoredStale;
        };
        if notification.state != NotificationState::Unread {
            return ReductionOutcome::IgnoredStale;
        }
        notification.state = NotificationState::Acknowledged;
        self.refresh_presentation(now_ms);
        self.revision = self.revision.saturating_add(1);
        ReductionOutcome::Applied {
            revision: self.revision,
        }
    }

    pub fn advance_presentation(&mut self, now_ms: u64) -> ReductionOutcome {
        if !self.refresh_presentation(now_ms) {
            return ReductionOutcome::IgnoredStale;
        }
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
            notification_priority(right)
                .cmp(&notification_priority(left))
                .then_with(|| right.occurred_at_ms.cmp(&left.occurred_at_ms))
                .then_with(|| left.id.cmp(&right.id))
        });
        SessionViewSnapshot {
            revision: self.revision,
            presentation: self.presentation.state,
            sessions,
            notifications,
        }
    }

    fn apply_session_event(&mut self, event: &NormalizedSessionEvent) -> Transition {
        let key = (event.provider.clone(), event.session_id.clone());
        match event.event_type {
            SessionEventKind::SessionStarted => {
                if let Some(session) = self.sessions.get_mut(&key) {
                    if event_order(event) <= session_order(session) || !session.ended {
                        return Transition::default();
                    }
                    session.ended = false;
                    session.current_turn_id = None;
                    session.updated_at_ms = event.occurred_at_ms;
                    session.last_event_id.clone_from(&event.event_id);
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
                if let Some(session) = self.sessions.get_mut(&key) {
                    if event_order(event) <= session_order(session) || session.ended {
                        return Transition::default();
                    }
                    session.ended = true;
                    session.updated_at_ms = event.occurred_at_ms;
                    session.last_event_id.clone_from(&event.event_id);
                } else {
                    let mut session = SessionRecord::new(event);
                    session.ended = true;
                    self.sessions.insert(key, session);
                }
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

        let session = self
            .sessions
            .entry(key)
            .or_insert_with(|| SessionRecord::new(event));
        let previous = session.turns.get(&turn_id);
        let Some(next_phase) = next_turn_phase(previous, event) else {
            return Transition::default();
        };
        session.turns.insert(
            turn_id.clone(),
            TurnRecord {
                phase: next_phase,
                updated_at_ms: event.occurred_at_ms,
                last_event_id: event.event_id.clone(),
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
                SessionEventKind::AttentionResolved
                    | SessionEventKind::TurnCompleted
                    | SessionEventKind::TurnFailed
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

    fn remove_superseded_terminal_notifications(&mut self, event: &NormalizedSessionEvent) {
        self.notifications.retain(|_, notification| {
            notification.provider != event.provider
                || notification.session_id != event.session_id
                || notification.turn_id != event.turn_id
                || !matches!(
                    notification.kind,
                    NotificationKind::Completion | NotificationKind::Failure
                )
        });
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

    fn refresh_presentation(&mut self, now_ms: u64) -> bool {
        let desired = self.desired_presentation();
        if desired == self.presentation.state {
            return false;
        }
        let can_interrupt = desired > self.presentation.state;
        let dwell_elapsed =
            now_ms.saturating_sub(self.presentation.since_ms) >= self.minimum_dwell_ms;
        if !can_interrupt && !dwell_elapsed {
            return false;
        }
        self.presentation = PresentationTracker {
            state: desired,
            since_ms: now_ms,
        };
        true
    }

    fn desired_presentation(&self) -> PresentationState {
        if self.notifications.values().any(|notification| {
            notification.kind == NotificationKind::Attention
                && notification.state != NotificationState::Resolved
        }) {
            return PresentationState::Waiting;
        }
        if self.notifications.values().any(|notification| {
            notification.kind == NotificationKind::Failure
                && notification.state == NotificationState::Unread
        }) {
            return PresentationState::Failed;
        }
        if self.notifications.values().any(|notification| {
            notification.kind == NotificationKind::Completion
                && notification.state == NotificationState::Unread
        }) {
            return PresentationState::Review;
        }
        if self
            .sessions
            .values()
            .any(|session| session.summary().phase == SessionPhase::Active)
        {
            return PresentationState::Running;
        }
        PresentationState::Idle
    }
}

impl Default for SessionReducer {
    fn default() -> Self {
        Self {
            revision: 0,
            sessions: BTreeMap::new(),
            notifications: BTreeMap::new(),
            recent_event_ids: VecDeque::new(),
            recent_event_set: BTreeSet::new(),
            presentation: PresentationTracker::default(),
            minimum_dwell_ms: DEFAULT_MINIMUM_DWELL_MS,
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
            last_event_id: event.event_id.clone(),
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
    previous: Option<&TurnRecord>,
    event: &NormalizedSessionEvent,
) -> Option<SessionPhase> {
    if let Some(previous) = previous {
        if event_order(event) <= turn_order(previous) {
            return None;
        }
        let previous_terminal = matches!(
            previous.phase,
            SessionPhase::Completed | SessionPhase::Failed
        );
        let incoming_terminal = matches!(
            event.event_type,
            SessionEventKind::TurnCompleted | SessionEventKind::TurnFailed
        );
        if previous_terminal && !incoming_terminal {
            return None;
        }
    }
    match event.event_type {
        SessionEventKind::TurnStarted => Some(SessionPhase::Active),
        SessionEventKind::AttentionRequired => Some(SessionPhase::Attention),
        SessionEventKind::AttentionResolved => Some(SessionPhase::Active),
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
            .map(|turn| (turn.updated_at_ms, &turn.last_event_id, current_id))
    });
    let incoming_order = (event.occurred_at_ms, &event.event_id, turn_id);
    if current_order.is_none_or(|current| incoming_order >= current) {
        session.current_turn_id = Some(turn_id.clone());
    }
    if event.occurred_at_ms >= session.updated_at_ms {
        session.updated_at_ms = event.occurred_at_ms;
        session.last_event_id.clone_from(&event.event_id);
        if event.project.is_some() {
            session.project.clone_from(&event.project);
        }
    }
}

fn event_order(event: &NormalizedSessionEvent) -> (u64, &EventId) {
    (event.occurred_at_ms, &event.event_id)
}

fn session_order(session: &SessionRecord) -> (u64, &EventId) {
    (session.updated_at_ms, &session.last_event_id)
}

fn turn_order(turn: &TurnRecord) -> (u64, &EventId) {
    (turn.updated_at_ms, &turn.last_event_id)
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

fn notification_priority(notification: &Notification) -> u8 {
    match (notification.kind, notification.state) {
        (NotificationKind::Attention, state) if state != NotificationState::Resolved => 3,
        (NotificationKind::Failure, NotificationState::Unread) => 2,
        (NotificationKind::Completion, NotificationState::Unread) => 1,
        _ => 0,
    }
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
            reducer.acknowledge_notification(&notification_id, 12),
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
            reducer.acknowledge_notification(&id, 11),
            ReductionOutcome::Applied { revision: 2 }
        );
        assert_eq!(
            reducer.acknowledge_notification(&id, 12),
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

    #[test]
    fn priority_interrupts_immediately_and_downgrade_honors_dwell() {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(100);
        reducer.reduce(event(
            "active",
            "turn_started",
            "session-active",
            Some("turn-1"),
            10,
        ));
        assert_eq!(reducer.snapshot().presentation, PresentationState::Running);
        reducer.reduce(event(
            "completed",
            "turn_completed",
            "session-complete",
            Some("turn-1"),
            20,
        ));
        assert_eq!(reducer.snapshot().presentation, PresentationState::Review);
        reducer.reduce(event(
            "failed",
            "turn_failed",
            "session-failed",
            Some("turn-1"),
            30,
        ));
        assert_eq!(reducer.snapshot().presentation, PresentationState::Failed);
        reducer.reduce(event(
            "attention",
            "attention_required",
            "session-waiting",
            Some("turn-1"),
            40,
        ));
        assert_eq!(reducer.snapshot().presentation, PresentationState::Waiting);
        reducer.reduce(event(
            "resolved",
            "attention_resolved",
            "session-waiting",
            Some("turn-1"),
            50,
        ));
        assert_eq!(reducer.snapshot().presentation, PresentationState::Waiting);
        assert_eq!(
            reducer.advance_presentation(139),
            ReductionOutcome::IgnoredStale
        );
        assert_eq!(
            reducer.advance_presentation(140),
            ReductionOutcome::Applied { revision: 6 }
        );
        assert_eq!(reducer.snapshot().presentation, PresentationState::Failed);
    }

    #[test]
    fn notification_order_is_independent_from_current_session() {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        reducer.reduce(event(
            "new-completion",
            "turn_completed",
            "session-complete",
            Some("turn-1"),
            50,
        ));
        reducer.reduce(event(
            "old-attention",
            "attention_required",
            "session-attention",
            Some("turn-1"),
            10,
        ));
        reducer.reduce(event(
            "active",
            "turn_started",
            "session-active",
            Some("turn-1"),
            100,
        ));
        let snapshot = reducer.snapshot();
        assert_eq!(snapshot.presentation, PresentationState::Waiting);
        assert_eq!(snapshot.notifications[0].kind, NotificationKind::Attention);
        assert_eq!(
            snapshot.notifications[0].session_id.as_str(),
            "session-attention"
        );
        assert_eq!(snapshot.notifications[1].kind, NotificationKind::Completion);
    }
}
