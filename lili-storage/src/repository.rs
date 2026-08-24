use diesel::prelude::*;
use diesel::result::QueryResult;
use diesel::sqlite::SqliteConnection;

use crate::models::{
    AppStateRow, InboundSpoolRow, JsonDocument, NewInboundSpool, NewPluginEvidence,
    PluginEvidenceRow,
};
use crate::schema::{app_state, inbound_spool, plugin_evidence};
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
            app_state::selected_pet_id.eq(&value.selected_pet_id),
            app_state::window_placement_json.eq(&value.window_placement_json),
            app_state::reducer_revision.eq(value.reducer_revision),
            app_state::reducer_json.eq(&value.reducer_json),
        ))
        .execute(connection)?;
    load_app_state(connection)
}

pub fn update_app_state_if_newer(
    connection: &mut SqliteConnection,
    value: &AppStateRow,
) -> QueryResult<bool> {
    let updated = diesel::update(
        app_state::table.find(value.id).filter(
            app_state::reducer_revision
                .lt(value.reducer_revision)
                .or(app_state::reducer_revision.eq(value.reducer_revision).and(
                    app_state::reducer_json
                        .eq(&value.reducer_json)
                        .or(app_state::reducer_json.is_null()),
                )),
        ),
    )
    .set((
        app_state::reducer_revision.eq(value.reducer_revision),
        app_state::reducer_json.eq(&value.reducer_json),
    ))
    .execute(connection)?;
    Ok(updated == 1)
}

pub fn update_selected_pet(
    connection: &mut SqliteConnection,
    selected_pet_id: Option<&str>,
) -> QueryResult<()> {
    diesel::update(app_state::table.find(1))
        .set(app_state::selected_pet_id.eq(selected_pet_id))
        .execute(connection)?;
    Ok(())
}

pub fn update_window_placement(
    connection: &mut SqliteConnection,
    window_placement_json: Option<&JsonDocument>,
) -> QueryResult<()> {
    diesel::update(app_state::table.find(1))
        .set(app_state::window_placement_json.eq(window_placement_json))
        .execute(connection)?;
    Ok(())
}

pub fn increment_spool_metrics(
    connection: &mut SqliteConnection,
    expired_drops: i64,
    limit_drops: i64,
    malformed_drops: i64,
) -> QueryResult<()> {
    diesel::update(app_state::table.find(1))
        .set((
            app_state::spool_expired_drops.eq(app_state::spool_expired_drops + expired_drops),
            app_state::spool_limit_drops.eq(app_state::spool_limit_drops + limit_drops),
            app_state::spool_malformed_drops.eq(app_state::spool_malformed_drops + malformed_drops),
        ))
        .execute(connection)?;
    Ok(())
}

pub fn insert_inbound_spool(
    connection: &mut SqliteConnection,
    value: &NewInboundSpool<'_>,
) -> QueryResult<InboundSpoolRow> {
    diesel::insert_into(inbound_spool::table)
        .values(value)
        .on_conflict((inbound_spool::provider, inbound_spool::event_id))
        .do_nothing()
        .execute(connection)?;
    inbound_spool::table
        .find((value.provider, value.event_id))
        .select(InboundSpoolRow::as_select())
        .first(connection)
}

pub fn insert_inbound_spool_if_retained(
    connection: &mut SqliteConnection,
    value: &NewInboundSpool<'_>,
) -> QueryResult<Option<InboundSpoolRow>> {
    diesel::insert_into(inbound_spool::table)
        .values(value)
        .on_conflict((inbound_spool::provider, inbound_spool::event_id))
        .do_nothing()
        .execute(connection)?;
    inbound_spool::table
        .find((value.provider, value.event_id))
        .select(InboundSpoolRow::as_select())
        .first(connection)
        .optional()
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
