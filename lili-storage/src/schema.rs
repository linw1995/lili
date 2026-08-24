diesel::table! {
    app_state (id) {
        id -> Integer,
        selected_pet_id -> Nullable<Text>,
        window_placement_json -> Nullable<Text>,
        reducer_revision -> BigInt,
        reducer_json -> Nullable<Text>,
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

diesel::allow_tables_to_appear_in_same_query!(app_state, inbound_spool, plugin_evidence,);
