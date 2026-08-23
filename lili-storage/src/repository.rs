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
