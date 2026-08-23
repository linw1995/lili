use diesel::prelude::*;
use diesel::result::QueryResult;
use diesel::sqlite::SqliteConnection;

use crate::models::{
    AppStateRow, InboundSpoolRow, LifecycleEventRow, NewInboundSpool, NewLifecycleEvent,
    NewNotification, NewPluginEvidence, NewRecentEvent, NewSession, NewTurn, NotificationRow,
    PluginEvidenceRow, RecentEventRow, SessionRow, TurnRow,
};
use crate::schema::{
    app_state, inbound_spool, lifecycle_events, notifications, plugin_evidence, recent_events,
    sessions, turns,
};
use crate::transaction::with_short_transaction;

pub fn load_app_state(connection: &mut SqliteConnection) -> QueryResult<AppStateRow> {
    app_state::table
        .find(1)
        .select(AppStateRow::as_select())
        .first(connection)
}

pub fn update_app_state(
    connection: &mut SqliteConnection,
    value: &AppStateRow,
) -> QueryResult<AppStateRow> {
    diesel::update(app_state::table.find(value.id))
        .set((
            app_state::schema_version.eq(value.schema_version),
            app_state::selected_pet_id.eq(&value.selected_pet_id),
            app_state::window_placement_json.eq(&value.window_placement_json),
            app_state::reducer_json.eq(&value.reducer_json),
            app_state::reducer_revision.eq(value.reducer_revision),
            app_state::presentation_state.eq(&value.presentation_state),
            app_state::presentation_since_ms.eq(value.presentation_since_ms),
            app_state::minimum_dwell_ms.eq(value.minimum_dwell_ms),
        ))
        .execute(connection)?;
    load_app_state(connection)
}

pub fn insert_session(
    connection: &mut SqliteConnection,
    value: &NewSession<'_>,
) -> QueryResult<SessionRow> {
    diesel::insert_into(sessions::table)
        .values(value)
        .execute(connection)?;
    sessions::table
        .find((value.provider, value.session_id))
        .select(SessionRow::as_select())
        .first(connection)
}

pub fn insert_turn(connection: &mut SqliteConnection, value: &NewTurn<'_>) -> QueryResult<TurnRow> {
    diesel::insert_into(turns::table)
        .values(value)
        .execute(connection)?;
    turns::table
        .find((value.provider, value.session_id, value.turn_id))
        .select(TurnRow::as_select())
        .first(connection)
}

pub fn insert_notification(
    connection: &mut SqliteConnection,
    value: &NewNotification<'_>,
) -> QueryResult<NotificationRow> {
    diesel::insert_into(notifications::table)
        .values(value)
        .execute(connection)?;
    notifications::table
        .find(value.id)
        .select(NotificationRow::as_select())
        .first(connection)
}

pub fn insert_recent_event(
    connection: &mut SqliteConnection,
    value: &NewRecentEvent<'_>,
) -> QueryResult<RecentEventRow> {
    diesel::insert_into(recent_events::table)
        .values(value)
        .execute(connection)?;
    recent_events::table
        .find((value.provider, value.event_id))
        .select(RecentEventRow::as_select())
        .first(connection)
}

pub fn insert_inbound_spool(
    connection: &mut SqliteConnection,
    value: &NewInboundSpool<'_>,
) -> QueryResult<InboundSpoolRow> {
    diesel::insert_into(inbound_spool::table)
        .values(value)
        .execute(connection)?;
    inbound_spool::table
        .find((value.provider, value.event_id))
        .select(InboundSpoolRow::as_select())
        .first(connection)
}

pub fn find_inbound_spool(
    connection: &mut SqliteConnection,
    provider: &str,
    event_id: &str,
) -> QueryResult<Option<InboundSpoolRow>> {
    inbound_spool::table
        .find((provider, event_id))
        .select(InboundSpoolRow::as_select())
        .first(connection)
        .optional()
}

pub fn list_inbound_spool(connection: &mut SqliteConnection) -> QueryResult<Vec<InboundSpoolRow>> {
    inbound_spool::table
        .order((
            inbound_spool::priority.asc(),
            inbound_spool::inserted_at_ms.asc(),
        ))
        .select(InboundSpoolRow::as_select())
        .load(connection)
}

pub fn recover_expired_inbound_spool_claims(
    connection: &mut SqliteConnection,
    now_ms: i64,
) -> QueryResult<usize> {
    diesel::update(
        inbound_spool::table
            .filter(inbound_spool::status.eq("claimed"))
            .filter(inbound_spool::lease_expires_at_ms.is_not_null())
            .filter(inbound_spool::lease_expires_at_ms.le(now_ms)),
    )
    .set((
        inbound_spool::status.eq("pending"),
        inbound_spool::claim_token.eq::<Option<&str>>(None),
        inbound_spool::claimed_at_ms.eq::<Option<i64>>(None),
        inbound_spool::lease_expires_at_ms.eq::<Option<i64>>(None),
    ))
    .execute(connection)
}

pub fn claim_inbound_spool(
    connection: &mut SqliteConnection,
    now_ms: i64,
    lease_ms: i64,
    claim_token: &str,
) -> QueryResult<Option<InboundSpoolRow>> {
    with_short_transaction(connection, |connection| {
        let Some(candidate) = inbound_spool::table
            .filter(inbound_spool::status.eq("pending"))
            .order((
                inbound_spool::priority.desc(),
                inbound_spool::inserted_at_ms.asc(),
            ))
            .select(InboundSpoolRow::as_select())
            .first(connection)
            .optional()?
        else {
            return Ok(None);
        };
        let updated = diesel::update(
            inbound_spool::table
                .find((&candidate.provider, &candidate.event_id))
                .filter(inbound_spool::status.eq("pending")),
        )
        .set((
            inbound_spool::status.eq("claimed"),
            inbound_spool::claim_token.eq(Some(claim_token)),
            inbound_spool::claimed_at_ms.eq(Some(now_ms)),
            inbound_spool::lease_expires_at_ms.eq(Some(now_ms.saturating_add(lease_ms))),
            inbound_spool::attempts.eq(inbound_spool::attempts + 1),
        ))
        .execute(connection)?;
        if updated != 1 {
            return Ok(None);
        }
        inbound_spool::table
            .find((&candidate.provider, &candidate.event_id))
            .select(InboundSpoolRow::as_select())
            .first(connection)
            .optional()
    })
}

pub fn delete_claimed_inbound_spool(
    connection: &mut SqliteConnection,
    provider: &str,
    event_id: &str,
    claim_token: &str,
) -> QueryResult<bool> {
    let deleted = diesel::delete(
        inbound_spool::table
            .find((provider, event_id))
            .filter(inbound_spool::status.eq("claimed"))
            .filter(inbound_spool::claim_token.eq(claim_token)),
    )
    .execute(connection)?;
    Ok(deleted == 1)
}

pub fn release_claimed_inbound_spool(
    connection: &mut SqliteConnection,
    provider: &str,
    event_id: &str,
    claim_token: &str,
) -> QueryResult<bool> {
    let updated = diesel::update(
        inbound_spool::table
            .find((provider, event_id))
            .filter(inbound_spool::status.eq("claimed"))
            .filter(inbound_spool::claim_token.eq(claim_token)),
    )
    .set((
        inbound_spool::status.eq("pending"),
        inbound_spool::claim_token.eq::<Option<&str>>(None),
        inbound_spool::claimed_at_ms.eq::<Option<i64>>(None),
        inbound_spool::lease_expires_at_ms.eq::<Option<i64>>(None),
    ))
    .execute(connection)?;
    Ok(updated == 1)
}

pub fn delete_inbound_spool(
    connection: &mut SqliteConnection,
    provider: &str,
    event_id: &str,
) -> QueryResult<bool> {
    let deleted =
        diesel::delete(inbound_spool::table.find((provider, event_id))).execute(connection)?;
    Ok(deleted == 1)
}

pub fn insert_lifecycle_event(
    connection: &mut SqliteConnection,
    value: &NewLifecycleEvent<'_>,
) -> QueryResult<LifecycleEventRow> {
    diesel::insert_into(lifecycle_events::table)
        .values(value)
        .execute(connection)?;
    lifecycle_events::table
        .find(value.event_id)
        .select(LifecycleEventRow::as_select())
        .first(connection)
}

pub fn load_plugin_evidence(
    connection: &mut SqliteConnection,
) -> QueryResult<Option<PluginEvidenceRow>> {
    plugin_evidence::table
        .find(1)
        .select(PluginEvidenceRow::as_select())
        .first(connection)
        .optional()
}

pub fn save_plugin_evidence(
    connection: &mut SqliteConnection,
    value: &NewPluginEvidence<'_>,
) -> QueryResult<PluginEvidenceRow> {
    diesel::insert_into(plugin_evidence::table)
        .values(value)
        .on_conflict(plugin_evidence::id)
        .do_update()
        .set((
            plugin_evidence::evidence_json.eq(value.evidence_json),
            plugin_evidence::updated_at_ms.eq(value.updated_at_ms),
        ))
        .execute(connection)?;
    plugin_evidence::table
        .find(value.id)
        .select(PluginEvidenceRow::as_select())
        .first(connection)
}
