use diesel::prelude::*;
use diesel::result::QueryResult;
use diesel::sqlite::SqliteConnection;

use crate::models::{
    AppStateRow, InboundSpoolRow, LifecycleEventRow, NewInboundSpool, NewLifecycleEvent,
    NewNotification, NewRecentEvent, NewSession, NewTurn, NotificationRow, PluginEvidenceRow,
    RecentEventRow, SessionRow, TurnRow,
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
