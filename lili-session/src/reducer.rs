use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt::Write as _,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    DisplayProjectContext, EventId, NormalizedSessionEvent, Notification, NotificationId,
    NotificationKind, NotificationState, PresentationState, ProviderId, SessionEventKind,
    SessionId, SessionPhase, SessionSummary, SessionViewSnapshot, TurnId,
};

const MAX_RECENT_EVENT_IDS: usize = 4096;
const MAX_PERSISTED_SESSIONS: usize = 128;
const MAX_PERSISTED_RETIRED_TURNS: usize = 16;
pub const DEFAULT_MINIMUM_DWELL_MS: u64 = 750;
pub const DEFAULT_ACTIVITY_REMINDER_DURATION_MS: u64 = 5_000;

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
    // ActivityReminder is a bounded event-driven display pulse, not persisted liveness.
    activity_reminder_until_ms: u64,
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
    retired_turn_ids: VecDeque<TurnId>,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionReducerState {
    revision: u64,
    sessions: Vec<PersistedSession>,
    notifications: Vec<Notification>,
}

impl SessionReducerState {
    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedSession {
    provider: ProviderId,
    id: SessionId,
    current_turn_id: Option<TurnId>,
    current_turn: Option<PersistedTurn>,
    retired_turn_ids: Vec<TurnId>,
    project: Option<DisplayProjectContext>,
    updated_at_ms: u64,
    last_event_id: EventId,
    ended: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedTurn {
    id: TurnId,
    phase: SessionPhase,
    updated_at_ms: u64,
    last_event_id: EventId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReducerRestoreError {
    TooManySessions,
    TooManyRetiredTurns,
    DuplicateSession,
    DuplicateNotification,
    DuplicateRetiredTurn,
    RetiredCurrentTurn,
    DuplicateSessionNotification,
    NotificationWithoutSession,
    InvalidCurrentTurn,
    InvalidNotificationState,
}

impl std::fmt::Display for ReducerRestoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::TooManySessions => "persisted reducer has too many sessions",
            Self::TooManyRetiredTurns => "persisted reducer has too many retired turns",
            Self::DuplicateSession => "persisted reducer has a duplicate session",
            Self::DuplicateNotification => "persisted reducer has a duplicate notification",
            Self::DuplicateRetiredTurn => "persisted reducer has a duplicate retired turn",
            Self::RetiredCurrentTurn => "persisted reducer retires the current turn",
            Self::DuplicateSessionNotification => {
                "persisted reducer has multiple notifications for one session"
            }
            Self::NotificationWithoutSession => {
                "persisted reducer has a notification without a session"
            }
            Self::InvalidCurrentTurn => "persisted reducer references an unknown current turn",
            Self::InvalidNotificationState => {
                "persisted reducer contains a non-unread notification"
            }
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ReducerRestoreError {}

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
        if self.is_duplicate_in_current_lifecycle(&event, &event_key) {
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

        if matches!(
            event.event_type,
            SessionEventKind::SessionStarted
                | SessionEventKind::TurnStarted
                | SessionEventKind::AttentionResolved
        ) {
            self.activity_reminder_until_ms = self.activity_reminder_until_ms.max(
                event
                    .occurred_at_ms
                    .saturating_add(DEFAULT_ACTIVITY_REMINDER_DURATION_MS),
            );
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

    pub fn persistent_state(&self) -> SessionReducerState {
        let unread_notification_priorities = self
            .notifications
            .values()
            .filter(|notification| notification.state == NotificationState::Unread)
            .fold(
                BTreeMap::<(ProviderId, SessionId), u8>::new(),
                |mut priorities, notification| {
                    let key = (
                        notification.provider.clone(),
                        notification.session_id.clone(),
                    );
                    let priority = notification_priority(notification);
                    priorities
                        .entry(key)
                        .and_modify(|current| *current = (*current).max(priority))
                        .or_insert(priority);
                    priorities
                },
            );
        let mut sessions = self.sessions.values().collect::<Vec<_>>();
        sessions.sort_by(|left, right| {
            session_persistence_priority(right, &unread_notification_priorities)
                .cmp(&session_persistence_priority(
                    left,
                    &unread_notification_priorities,
                ))
                .then_with(|| right.updated_at_ms.cmp(&left.updated_at_ms))
                .then_with(|| left.provider.cmp(&right.provider))
                .then_with(|| left.id.cmp(&right.id))
        });
        let sessions = sessions
            .into_iter()
            .take(MAX_PERSISTED_SESSIONS)
            .map(PersistedSession::from_record)
            .collect::<Vec<_>>();
        let persisted_session_keys = sessions
            .iter()
            .map(|session| (session.provider.clone(), session.id.clone()))
            .collect::<BTreeSet<_>>();

        let mut state_notifications = BTreeMap::new();
        for notification in self
            .notifications
            .values()
            .filter(|notification| notification.state == NotificationState::Unread)
            .filter(|notification| {
                persisted_session_keys.contains(&(
                    notification.provider.clone(),
                    notification.session_id.clone(),
                ))
            })
        {
            let key = (
                notification.provider.clone(),
                notification.session_id.clone(),
            );
            let replace = state_notifications
                .get(&key)
                .is_none_or(|current: &Notification| {
                    notification_order(notification) > notification_order(current)
                });
            if replace {
                state_notifications.insert(key, notification.clone());
            }
        }
        let mut notifications = state_notifications.into_values().collect::<Vec<_>>();
        notifications.sort_by(|left, right| {
            notification_priority(right)
                .cmp(&notification_priority(left))
                .then_with(|| right.occurred_at_ms.cmp(&left.occurred_at_ms))
                .then_with(|| left.id.cmp(&right.id))
        });
        SessionReducerState {
            revision: self.revision,
            sessions,
            notifications,
        }
    }

    pub fn from_persistent_state(state: SessionReducerState) -> Result<Self, ReducerRestoreError> {
        if state.sessions.len() > MAX_PERSISTED_SESSIONS {
            return Err(ReducerRestoreError::TooManySessions);
        }
        let mut sessions = BTreeMap::new();
        for persisted in state.sessions {
            let record = persisted.into_record()?;
            let key = (record.provider.clone(), record.id.clone());
            if sessions.insert(key, record).is_some() {
                return Err(ReducerRestoreError::DuplicateSession);
            }
        }
        let mut notifications = BTreeMap::new();
        let mut notification_sessions = BTreeSet::new();
        for notification in state.notifications {
            if notification.state != NotificationState::Unread {
                return Err(ReducerRestoreError::InvalidNotificationState);
            }
            let session_key = (
                notification.provider.clone(),
                notification.session_id.clone(),
            );
            if !sessions.contains_key(&session_key) {
                return Err(ReducerRestoreError::NotificationWithoutSession);
            }
            if !notification_sessions.insert(session_key) {
                return Err(ReducerRestoreError::DuplicateSessionNotification);
            }
            if notifications
                .insert(notification.id.clone(), notification)
                .is_some()
            {
                return Err(ReducerRestoreError::DuplicateNotification);
            }
        }
        let mut reducer = Self {
            revision: state.revision,
            sessions,
            notifications,
            recent_event_ids: VecDeque::new(),
            recent_event_set: BTreeSet::new(),
            presentation: PresentationTracker::default(),
            minimum_dwell_ms: DEFAULT_MINIMUM_DWELL_MS,
            activity_reminder_until_ms: 0,
        };
        reducer.presentation.state = reducer.desired_presentation(0);
        Ok(reducer)
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
        let starts_new_turn = previous.is_none()
            && session
                .current_turn_id
                .as_ref()
                .is_some_and(|current| current != &turn_id);
        if starts_new_turn && session.retired_turn_ids.contains(&turn_id) {
            return Transition::default();
        }
        if previous.is_none()
            && event.event_id != session.last_event_id
            && event_order(event) <= session_order(session)
        {
            return Transition::default();
        }
        let Some(next_phase) = next_turn_phase(previous, event) else {
            return Transition::default();
        };
        if starts_new_turn && let Some(current_turn_id) = session.current_turn_id.clone() {
            remember_retired_turn(session, current_turn_id);
        }
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
        if !self.recent_event_set.insert(event.clone()) {
            self.recent_event_ids
                .retain(|remembered| remembered != &event);
        }
        self.recent_event_ids.push_back(event);
        while self.recent_event_ids.len() > MAX_RECENT_EVENT_IDS {
            if let Some(expired) = self.recent_event_ids.pop_front() {
                self.recent_event_set.remove(&expired);
            }
        }
    }

    fn is_duplicate_in_current_lifecycle(
        &self,
        event: &NormalizedSessionEvent,
        event_key: &(ProviderId, EventId),
    ) -> bool {
        if !self.recent_event_set.contains(event_key) {
            return false;
        }
        let session = self
            .sessions
            .get(&(event.provider.clone(), event.session_id.clone()));
        match event.event_type {
            SessionEventKind::SessionStarted => session.is_some_and(|session| !session.ended),
            SessionEventKind::SessionEnded => session.is_some_and(|session| session.ended),
            _ => true,
        }
    }

    fn refresh_presentation(&mut self, now_ms: u64) -> bool {
        let desired = self.desired_presentation(now_ms);
        if desired == self.presentation.state {
            return false;
        }
        let expired_activity_reminder = self.presentation.state
            == PresentationState::ActivityReminder
            && desired == PresentationState::Idle
            && self.activity_reminder_until_ms <= now_ms;
        let can_interrupt = desired > self.presentation.state
            || desired == PresentationState::ActivityReminder
            || expired_activity_reminder;
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

    fn desired_presentation(&self, now_ms: u64) -> PresentationState {
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
        if self.activity_reminder_until_ms > now_ms {
            return PresentationState::ActivityReminder;
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
            activity_reminder_until_ms: 0,
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
            retired_turn_ids: VecDeque::new(),
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

impl PersistedSession {
    fn from_record(record: &SessionRecord) -> Self {
        let current_turn = record.current_turn_id.as_ref().and_then(|turn_id| {
            record.turns.get(turn_id).map(|turn| PersistedTurn {
                id: turn_id.clone(),
                phase: turn.phase,
                updated_at_ms: turn.updated_at_ms,
                last_event_id: turn.last_event_id.clone(),
            })
        });
        Self {
            provider: record.provider.clone(),
            id: record.id.clone(),
            current_turn_id: current_turn.as_ref().map(|turn| turn.id.clone()),
            current_turn,
            retired_turn_ids: record.retired_turn_ids.iter().cloned().collect(),
            project: record.project.clone(),
            updated_at_ms: record.updated_at_ms,
            last_event_id: record.last_event_id.clone(),
            ended: record.ended,
        }
    }

    fn into_record(self) -> Result<SessionRecord, ReducerRestoreError> {
        let mut turns = BTreeMap::new();
        if let Some(turn) = self.current_turn {
            let record = TurnRecord {
                phase: turn.phase,
                updated_at_ms: turn.updated_at_ms,
                last_event_id: turn.last_event_id,
            };
            turns.insert(turn.id, record);
        }
        if self.retired_turn_ids.len() > MAX_PERSISTED_RETIRED_TURNS {
            return Err(ReducerRestoreError::TooManyRetiredTurns);
        }
        let mut retired_turn_ids = VecDeque::new();
        for turn_id in self.retired_turn_ids {
            if self.current_turn_id.as_ref() == Some(&turn_id) {
                return Err(ReducerRestoreError::RetiredCurrentTurn);
            }
            if retired_turn_ids.contains(&turn_id) {
                return Err(ReducerRestoreError::DuplicateRetiredTurn);
            }
            retired_turn_ids.push_back(turn_id);
        }
        if self
            .current_turn_id
            .as_ref()
            .is_some_and(|current| !turns.contains_key(current))
        {
            return Err(ReducerRestoreError::InvalidCurrentTurn);
        }
        Ok(SessionRecord {
            provider: self.provider,
            id: self.id,
            current_turn_id: self.current_turn_id,
            turns,
            retired_turn_ids,
            project: self.project,
            updated_at_ms: self.updated_at_ms,
            last_event_id: self.last_event_id,
            ended: self.ended,
        })
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

fn remember_retired_turn(session: &mut SessionRecord, turn_id: TurnId) {
    if let Some(index) = session
        .retired_turn_ids
        .iter()
        .position(|retired| retired == &turn_id)
    {
        session.retired_turn_ids.remove(index);
    }
    session.retired_turn_ids.push_back(turn_id);
    while session.retired_turn_ids.len() > MAX_PERSISTED_RETIRED_TURNS {
        session.retired_turn_ids.pop_front();
    }
}

fn event_order(event: &NormalizedSessionEvent) -> (u64, &EventId) {
    (event.occurred_at_ms, &event.event_id)
}

fn session_order(session: &SessionRecord) -> (u64, &EventId) {
    (session.updated_at_ms, &session.last_event_id)
}

fn session_persistence_priority(
    session: &SessionRecord,
    unread_notification_priorities: &BTreeMap<(ProviderId, SessionId), u8>,
) -> (u8, u8) {
    let notification_priority = unread_notification_priorities
        .get(&(session.provider.clone(), session.id.clone()))
        .copied()
        .unwrap_or_default();
    let state_priority = match session.summary().phase {
        SessionPhase::Attention => 2,
        SessionPhase::Active => 1,
        _ => 0,
    };
    (notification_priority, state_priority)
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

fn notification_order(notification: &Notification) -> (u8, u64, &NotificationId) {
    (
        notification_priority(notification),
        notification.occurred_at_ms,
        &notification.id,
    )
}

#[cfg(test)]
mod tests {
    use crate::{ProviderCapabilitiesInputV1, ProviderInputV1, normalize_provider_input};

    use super::*;

    #[test]
    fn restore_errors_have_safe_public_messages() {
        let errors = [
            ReducerRestoreError::TooManySessions,
            ReducerRestoreError::TooManyRetiredTurns,
            ReducerRestoreError::DuplicateSession,
            ReducerRestoreError::DuplicateNotification,
            ReducerRestoreError::DuplicateRetiredTurn,
            ReducerRestoreError::RetiredCurrentTurn,
            ReducerRestoreError::DuplicateSessionNotification,
            ReducerRestoreError::NotificationWithoutSession,
            ReducerRestoreError::InvalidCurrentTurn,
            ReducerRestoreError::InvalidNotificationState,
        ];

        for error in errors {
            assert!(error.to_string().starts_with("persisted reducer "));
        }
    }

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
    fn default_reducer_starts_without_a_live_session() {
        let snapshot = SessionReducer::default().snapshot();
        assert_eq!(snapshot.revision, 0);
        assert_eq!(snapshot.presentation, PresentationState::Idle);
        assert!(snapshot.sessions.is_empty());
    }

    #[test]
    fn session_start_keeps_turn_idle_and_starts_an_activity_reminder() {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        reducer.reduce(event(
            "session-start",
            "session_started",
            "session-1",
            None,
            1,
        ));
        let snapshot = reducer.snapshot();
        assert_eq!(snapshot.presentation, PresentationState::ActivityReminder);
        assert_eq!(snapshot.sessions[0].phase, SessionPhase::Idle);
        assert_eq!(
            reducer.advance_presentation(DEFAULT_ACTIVITY_REMINDER_DURATION_MS + 1),
            ReductionOutcome::Applied { revision: 2 }
        );
        assert_eq!(reducer.snapshot().presentation, PresentationState::Idle);
    }

    #[test]
    fn activity_reminder_returns_to_idle_after_duration() {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        reducer.reduce(event(
            "turn-start",
            "turn_started",
            "session-1",
            Some("turn-1"),
            1,
        ));
        assert_eq!(
            reducer.snapshot().presentation,
            PresentationState::ActivityReminder
        );

        assert_eq!(
            reducer.advance_presentation(DEFAULT_ACTIVITY_REMINDER_DURATION_MS),
            ReductionOutcome::IgnoredStale
        );
        assert_eq!(
            reducer.snapshot().presentation,
            PresentationState::ActivityReminder
        );
        assert_eq!(
            reducer.advance_presentation(DEFAULT_ACTIVITY_REMINDER_DURATION_MS + 1),
            ReductionOutcome::Applied { revision: 2 }
        );
        assert_eq!(reducer.snapshot().presentation, PresentationState::Idle);
        assert_eq!(reducer.snapshot().sessions[0].phase, SessionPhase::Active);
    }

    #[test]
    fn session_end_does_not_change_the_activity_reminder() {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        reducer.reduce(event(
            "turn-start",
            "turn_started",
            "session-1",
            Some("turn-1"),
            1,
        ));
        reducer.reduce(event("session-end", "session_ended", "session-1", None, 2));

        let snapshot = reducer.snapshot();
        assert_eq!(snapshot.presentation, PresentationState::ActivityReminder);
        assert_eq!(snapshot.sessions[0].phase, SessionPhase::Ended);
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
    fn resumed_session_can_end_again_with_the_same_overlap_identity() {
        let mut reducer = SessionReducer::default();
        let end = event("event-end", "session_ended", "session-1", None, 10);
        assert_eq!(
            reducer.reduce(end.clone()),
            ReductionOutcome::Applied { revision: 1 }
        );
        assert_eq!(reducer.reduce(end.clone()), ReductionOutcome::Duplicate);
        assert_eq!(
            reducer.reduce(event(
                "event-resume",
                "session_started",
                "session-1",
                None,
                20,
            )),
            ReductionOutcome::Applied { revision: 2 }
        );
        assert_eq!(
            reducer.reduce(event(
                "event-attention",
                "attention_required",
                "session-1",
                Some("turn-1"),
                21,
            )),
            ReductionOutcome::Applied { revision: 3 }
        );

        let second_end = NormalizedSessionEvent {
            occurred_at_ms: 30,
            ..end
        };
        assert_eq!(
            reducer.reduce(second_end.clone()),
            ReductionOutcome::Applied { revision: 4 }
        );
        assert_eq!(reducer.reduce(second_end), ReductionOutcome::Duplicate);
        let snapshot = reducer.snapshot();
        assert_eq!(snapshot.sessions[0].phase, SessionPhase::Ended);
        assert_eq!(snapshot.notifications[0].state, NotificationState::Resolved);
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
        assert_eq!(
            reducer.snapshot().presentation,
            PresentationState::ActivityReminder
        );
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

    #[test]
    fn persistence_restores_reducer_metadata_and_only_unread_notifications() {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        let failed = event(
            "failed",
            "turn_failed",
            "session-failed",
            Some("turn-1"),
            10,
        );
        reducer.reduce(failed.clone());
        let failed_notification = reducer.snapshot().notifications[0].id.clone();
        reducer.acknowledge_notification(&failed_notification, 11);
        let attention = event(
            "attention",
            "attention_required",
            "session-attention",
            Some("turn-1"),
            20,
        );
        reducer.reduce(attention);

        let restored = SessionReducer::from_persistent_state(reducer.persistent_state()).unwrap();
        let snapshot = restored.snapshot();
        assert_eq!(snapshot.revision, reducer.snapshot().revision);
        assert_eq!(snapshot.sessions.len(), 2);
        assert_eq!(snapshot.notifications.len(), 1);
        assert_eq!(snapshot.notifications[0].kind, NotificationKind::Attention);

        let mut restored = restored;
        assert_eq!(restored.reduce(failed), ReductionOutcome::IgnoredStale);
    }

    #[test]
    fn persistence_keeps_one_current_turn_and_state_notification_per_session() {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        reducer.reduce(event(
            "attention",
            "attention_required",
            "session-1",
            Some("turn-1"),
            10,
        ));
        reducer.reduce(event(
            "completed",
            "turn_completed",
            "session-1",
            Some("turn-1"),
            20,
        ));
        reducer.reduce(event(
            "next-turn",
            "turn_started",
            "session-1",
            Some("turn-2"),
            30,
        ));

        let persisted = reducer.persistent_state();
        assert_eq!(persisted.sessions.len(), 1);
        assert_eq!(
            persisted.sessions[0]
                .current_turn
                .as_ref()
                .map(|turn| turn.id.as_str()),
            Some("turn-2")
        );
        assert_eq!(persisted.notifications.len(), 1);
        assert_eq!(persisted.notifications[0].event_id.as_str(), "completed");

        let mut restored = SessionReducer::from_persistent_state(persisted).unwrap();
        assert_eq!(
            restored.reduce(event(
                "completed",
                "turn_completed",
                "session-1",
                Some("turn-1"),
                20,
            )),
            ReductionOutcome::IgnoredStale
        );
    }

    #[test]
    fn persistence_rejects_replayed_retired_turns_after_restart() {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        reducer.reduce(event(
            "started-1",
            "turn_started",
            "session-1",
            Some("turn-1"),
            10,
        ));
        reducer.reduce(event(
            "completed-1",
            "turn_completed",
            "session-1",
            Some("turn-1"),
            20,
        ));
        reducer.reduce(event(
            "started-2",
            "turn_started",
            "session-1",
            Some("turn-2"),
            30,
        ));

        let mut restored =
            SessionReducer::from_persistent_state(reducer.persistent_state()).unwrap();
        assert_eq!(
            restored.reduce(event(
                "completed-1-retry",
                "turn_completed",
                "session-1",
                Some("turn-1"),
                100,
            )),
            ReductionOutcome::IgnoredStale
        );
        assert_eq!(
            restored.snapshot().sessions[0]
                .current_turn_id
                .as_ref()
                .map(TurnId::as_str),
            Some("turn-2")
        );
    }

    #[test]
    fn persistence_keeps_an_older_unread_notification_when_newer_one_is_acknowledged() {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        reducer.reduce(event(
            "completed-1",
            "turn_completed",
            "session-1",
            Some("turn-1"),
            10,
        ));
        reducer.reduce(event(
            "started-2",
            "turn_started",
            "session-1",
            Some("turn-2"),
            20,
        ));
        reducer.reduce(event(
            "completed-2",
            "turn_completed",
            "session-1",
            Some("turn-2"),
            30,
        ));
        let newer_notification = reducer
            .snapshot()
            .notifications
            .iter()
            .find(|notification| notification.event_id.as_str() == "completed-2")
            .map(|notification| notification.id.clone())
            .unwrap();
        reducer.acknowledge_notification(&newer_notification, 31);

        let persisted = reducer.persistent_state();
        assert_eq!(persisted.notifications.len(), 1);
        assert_eq!(persisted.notifications[0].event_id.as_str(), "completed-1");
    }

    #[test]
    fn persistence_keeps_the_notification_that_drives_presentation() {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        reducer.reduce(event(
            "attention",
            "attention_required",
            "session-1",
            Some("turn-1"),
            10,
        ));
        reducer.reduce(event(
            "started-2",
            "turn_started",
            "session-1",
            Some("turn-2"),
            20,
        ));
        reducer.reduce(event(
            "completed-2",
            "turn_completed",
            "session-1",
            Some("turn-2"),
            30,
        ));

        let persisted = reducer.persistent_state();
        assert_eq!(persisted.notifications.len(), 1);
        assert_eq!(persisted.notifications[0].event_id.as_str(), "attention");

        let restored = SessionReducer::from_persistent_state(persisted).unwrap();
        assert_eq!(restored.snapshot().presentation, PresentationState::Waiting);
    }

    #[test]
    fn persistence_keeps_notifications_only_for_persisted_sessions() {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        for index in 0..(MAX_PERSISTED_SESSIONS + 12) {
            reducer.reduce(event(
                &format!("event-{index}"),
                "turn_completed",
                &format!("session-{index}"),
                Some("turn-1"),
                index as u64,
            ));
        }
        let restored = SessionReducer::from_persistent_state(reducer.persistent_state()).unwrap();
        let snapshot = restored.snapshot();
        assert_eq!(snapshot.sessions.len(), MAX_PERSISTED_SESSIONS);
        assert_eq!(snapshot.notifications.len(), MAX_PERSISTED_SESSIONS);
        assert!(
            snapshot
                .notifications
                .iter()
                .all(|notification| notification.session_id.as_str() != "session-0")
        );
    }

    #[test]
    fn persistence_retains_state_driving_sessions_and_recomputes_presentation() {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        reducer.reduce(event(
            "active-event",
            "turn_started",
            "old-active-session",
            Some("turn-1"),
            1,
        ));
        for index in 0..(MAX_PERSISTED_SESSIONS + 12) {
            reducer.reduce(event(
                &format!("completed-{index}"),
                "session_ended",
                &format!("ended-session-{index}"),
                None,
                100 + index as u64,
            ));
        }

        let persisted = reducer.persistent_state();
        assert!(
            persisted
                .sessions
                .iter()
                .any(|session| session.id.as_str() == "old-active-session")
        );

        let mut restored = SessionReducer::from_persistent_state(persisted).unwrap();
        assert_eq!(restored.snapshot().presentation, PresentationState::Idle);
        assert!(matches!(
            restored.reduce(event(
                "active-refresh",
                "turn_started",
                "old-active-session",
                Some("turn-1"),
                10_000,
            )),
            ReductionOutcome::Applied { .. }
        ));
        assert_eq!(
            restored.snapshot().presentation,
            PresentationState::ActivityReminder
        );
    }

    #[test]
    fn persistence_retains_sessions_with_presentation_driving_notifications() {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        for index in 0..MAX_PERSISTED_SESSIONS {
            reducer.reduce(event(
                &format!("active-{index}"),
                "turn_started",
                &format!("active-session-{index}"),
                Some("turn-1"),
                index as u64,
            ));
        }
        reducer.reduce(event(
            "failed",
            "turn_failed",
            "failed-session",
            Some("turn-1"),
            10_000,
        ));

        let persisted = reducer.persistent_state();
        assert_eq!(persisted.sessions.len(), MAX_PERSISTED_SESSIONS);
        assert!(
            persisted
                .sessions
                .iter()
                .any(|session| session.id.as_str() == "failed-session")
        );
        let restored = SessionReducer::from_persistent_state(persisted).unwrap();
        assert_eq!(restored.snapshot().presentation, PresentationState::Failed);
    }

    #[test]
    fn persistence_rejects_multiple_notifications_for_one_session() {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        reducer.reduce(event(
            "completion",
            "turn_completed",
            "session-1",
            Some("turn-1"),
            10,
        ));
        let mut persisted = reducer.persistent_state();
        persisted
            .notifications
            .push(persisted.notifications[0].clone());

        assert!(matches!(
            SessionReducer::from_persistent_state(persisted),
            Err(ReducerRestoreError::DuplicateSessionNotification)
        ));
    }
}
