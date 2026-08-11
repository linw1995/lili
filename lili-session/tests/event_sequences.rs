use lili_session::{
    NormalizationError, NormalizedSessionEvent, NotificationKind, PresentationState,
    ProviderCapabilitiesInputV1, ProviderInputV1, ReductionOutcome, SessionPhase, SessionReducer,
    normalize_json, normalize_provider_input,
};

fn event(
    provider: &str,
    event_id: &str,
    event_type: &str,
    session_id: &str,
    turn_id: Option<&str>,
    occurred_at_ms: u64,
) -> NormalizedSessionEvent {
    normalize_provider_input(ProviderInputV1 {
        version: 1,
        provider: Some(provider.to_owned()),
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

fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    if items.is_empty() {
        return vec![Vec::new()];
    }
    let mut result = Vec::new();
    for index in 0..items.len() {
        let mut remaining = items.to_vec();
        let first = remaining.remove(index);
        for mut suffix in permutations(&remaining) {
            let mut permutation = vec![first.clone()];
            permutation.append(&mut suffix);
            result.push(permutation);
        }
    }
    result
}

#[test]
fn required_field_validation_is_table_driven_and_payload_safe() {
    let base = serde_json::json!({
        "version": 1,
        "provider": "codex",
        "type": "turn_started",
        "sessionId": "session-1",
        "turnId": "turn-1",
        "occurredAtMs": 10,
        "summary": "secret-marker"
    });
    let cases = [
        ("provider", NormalizationError::MissingField("provider")),
        ("type", NormalizationError::MissingField("event type")),
        (
            "sessionId",
            NormalizationError::MissingField("session identity"),
        ),
        ("turnId", NormalizationError::MissingField("turn identity")),
        (
            "occurredAtMs",
            NormalizationError::MissingField("occurrence time"),
        ),
    ];

    for (field, expected) in cases {
        let mut input = base.clone();
        input.as_object_mut().unwrap().remove(field);
        let error = normalize_json(&serde_json::to_vec(&input).unwrap()).unwrap_err();
        assert_eq!(error, expected, "field {field}");
        assert!(!error.to_string().contains("secret-marker"));
    }
}

#[test]
fn additive_provider_fields_do_not_change_normalized_identity() {
    let base = serde_json::json!({
        "version": 1,
        "provider": "codex",
        "type": "turn_completed",
        "sessionId": "session-1",
        "turnId": "turn-1",
        "occurredAtMs": 10
    });
    let mut extended = base.clone();
    extended["providerFuture"] = serde_json::json!({"nested": [1, 2, 3]});
    let base = normalize_json(&serde_json::to_vec(&base).unwrap()).unwrap();
    let extended = normalize_json(&serde_json::to_vec(&extended).unwrap()).unwrap();
    assert_eq!(base, extended);
}

#[test]
fn duplicate_and_stale_sequence_property_holds_for_every_delivery_order() {
    let active = event(
        "codex",
        "active",
        "turn_started",
        "session-1",
        Some("turn-1"),
        10,
    );
    let completed = event(
        "codex",
        "completed",
        "turn_completed",
        "session-1",
        Some("turn-1"),
        20,
    );
    for sequence in permutations(&[active, completed.clone(), completed]) {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        for event in sequence {
            reducer.reduce(event);
        }
        let snapshot = reducer.snapshot();
        assert_eq!(snapshot.sessions[0].phase, SessionPhase::Completed);
        assert_eq!(snapshot.notifications.len(), 1);
        assert_eq!(snapshot.notifications[0].kind, NotificationKind::Completion);
    }
}

#[test]
fn reordered_terminal_conflicts_choose_the_stable_latest_event() {
    let completed = event(
        "codex",
        "terminal-a",
        "turn_completed",
        "session-1",
        Some("turn-1"),
        20,
    );
    let failed = event(
        "codex",
        "terminal-b",
        "turn_failed",
        "session-1",
        Some("turn-1"),
        30,
    );
    for sequence in permutations(&[completed, failed]) {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        for event in sequence {
            reducer.reduce(event);
        }
        let snapshot = reducer.snapshot();
        assert_eq!(snapshot.sessions[0].phase, SessionPhase::Failed);
        assert_eq!(snapshot.notifications.len(), 1);
        assert_eq!(snapshot.notifications[0].kind, NotificationKind::Failure);
    }
}

#[test]
fn session_end_tombstone_wins_over_an_older_start_in_every_order() {
    let started = event("codex", "started", "session_started", "session-1", None, 10);
    let ended = event("codex", "ended", "session_ended", "session-1", None, 20);
    for sequence in permutations(&[started, ended]) {
        let mut reducer = SessionReducer::default();
        for event in sequence {
            reducer.reduce(event);
        }
        assert_eq!(reducer.snapshot().sessions[0].phase, SessionPhase::Ended);
    }
}

#[test]
fn concurrent_session_priority_property_is_permutation_invariant() {
    let events = [
        event(
            "codex",
            "active",
            "turn_started",
            "session-active",
            Some("turn-1"),
            40,
        ),
        event(
            "codex",
            "completed",
            "turn_completed",
            "session-complete",
            Some("turn-1"),
            30,
        ),
        event(
            "codex",
            "failed",
            "turn_failed",
            "session-failed",
            Some("turn-1"),
            20,
        ),
        event(
            "codex",
            "attention",
            "attention_required",
            "session-attention",
            Some("turn-1"),
            10,
        ),
    ];
    for sequence in permutations(&events) {
        let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
        for event in sequence {
            reducer.reduce(event);
        }
        let snapshot = reducer.snapshot();
        assert_eq!(snapshot.presentation, PresentationState::Waiting);
        assert_eq!(
            snapshot
                .notifications
                .iter()
                .map(|notification| notification.kind)
                .collect::<Vec<_>>(),
            vec![
                NotificationKind::Attention,
                NotificationKind::Failure,
                NotificationKind::Completion,
            ]
        );
    }
}

#[test]
fn provider_identity_is_part_of_deduplication_and_session_keys() {
    let mut reducer = SessionReducer::with_minimum_dwell_ms(0);
    for provider in ["codex", "chatgpt"] {
        assert!(matches!(
            reducer.reduce(event(
                provider,
                "shared-event",
                "turn_started",
                "shared-session",
                Some("turn-1"),
                10,
            )),
            ReductionOutcome::Applied { .. }
        ));
    }
    assert_eq!(reducer.snapshot().sessions.len(), 2);
}
