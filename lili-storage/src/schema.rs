diesel::table! {
    app_state (id) {
        id -> Integer,
        schema_version -> Integer,
        selected_pet_id -> Nullable<Text>,
        window_placement_json -> Nullable<Text>,
        reducer_revision -> BigInt,
        presentation_state -> Text,
        presentation_since_ms -> BigInt,
        minimum_dwell_ms -> BigInt,
    }
}

diesel::table! {
    sessions (provider, session_id) {
        provider -> Text,
        session_id -> Text,
        current_turn_id -> Nullable<Text>,
        project_json -> Nullable<Text>,
        updated_at_ms -> BigInt,
        last_event_id -> Text,
        ended -> Integer,
    }
}

diesel::table! {
    turns (provider, session_id, turn_id) {
        provider -> Text,
        session_id -> Text,
        turn_id -> Text,
        phase -> Text,
        updated_at_ms -> BigInt,
        last_event_id -> Text,
    }
}

diesel::table! {
    notifications (id) {
        id -> Text,
        provider -> Text,
        event_id -> Text,
        session_id -> Text,
        turn_id -> Nullable<Text>,
        kind -> Text,
        state -> Text,
        occurred_at_ms -> BigInt,
        project_json -> Nullable<Text>,
        summary_json -> Nullable<Text>,
    }
}

diesel::table! {
    recent_events (provider, event_id) {
        provider -> Text,
        event_id -> Text,
        observed_at_ms -> BigInt,
    }
}

diesel::table! {
    inbound_spool (provider, event_id) {
        provider -> Text,
        event_id -> Text,
        payload_json -> Text,
        priority -> Integer,
        occurred_at_ms -> BigInt,
        inserted_at_ms -> BigInt,
        status -> Text,
        claim_token -> Nullable<Text>,
        claimed_at_ms -> Nullable<BigInt>,
        lease_expires_at_ms -> Nullable<BigInt>,
        attempts -> Integer,
    }
}

diesel::table! {
    plugin_evidence (id) {
        id -> Integer,
        evidence_json -> Text,
        updated_at_ms -> BigInt,
    }
}

diesel::table! {
    lifecycle_events (event_id) {
        event_id -> Text,
        entity_type -> Text,
        entity_id -> Text,
        event_type -> Text,
        source -> Text,
        occurred_at_ms -> BigInt,
        previous_state -> Nullable<Text>,
        current_state -> Nullable<Text>,
        details_json -> Nullable<Text>,
        error_json -> Nullable<Text>,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    app_state,
    sessions,
    turns,
    notifications,
    recent_events,
    inbound_spool,
    plugin_evidence,
    lifecycle_events,
);
