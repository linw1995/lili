use std::fmt;

use diesel::backend::Backend;
use diesel::deserialize::{self, FromSql};
use diesel::prelude::*;
use diesel::serialize::{self, IsNull, Output, ToSql};
use diesel::sql_types::Text;
use diesel::sqlite::Sqlite;
use serde::{Deserialize, Serialize};

use crate::schema::{
    app_state, inbound_spool, lifecycle_events, notifications, plugin_evidence, recent_events,
    sessions, turns,
};

const MAX_JSON_DOCUMENT_BYTES: usize = 64 * 1024;

#[derive(
    Clone, Debug, Eq, PartialEq, Serialize, Deserialize, diesel::AsExpression, diesel::FromSqlRow,
)]
#[diesel(sql_type = Text)]
pub struct JsonDocument(String);

impl JsonDocument {
    pub fn parse(value: impl Into<String>) -> Result<Self, JsonDocumentError> {
        let value = value.into();
        if value.len() > MAX_JSON_DOCUMENT_BYTES {
            return Err(JsonDocumentError::TooLarge);
        }
        serde_json::from_str::<serde_json::Value>(&value)
            .map_err(|_| JsonDocumentError::Malformed)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for JsonDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum JsonDocumentError {
    #[error("JSON document is malformed")]
    Malformed,
    #[error("JSON document exceeds 64 KiB")]
    TooLarge,
}

impl ToSql<Text, Sqlite> for JsonDocument {
    fn to_sql<'b>(&'b self, output: &mut Output<'b, '_, Sqlite>) -> serialize::Result {
        output.set_value(self.as_str());
        Ok(IsNull::No)
    }
}

impl<DB> FromSql<Text, DB> for JsonDocument
where
    DB: Backend,
    String: FromSql<Text, DB>,
{
    fn from_sql(bytes: DB::RawValue<'_>) -> deserialize::Result<Self> {
        let value = String::from_sql(bytes)?;
        Self::parse(value)
            .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>)
    }
}

#[derive(Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = app_state)]
#[diesel(check_for_backend(Sqlite))]
pub struct AppStateRow {
    pub id: i32,
    pub schema_version: i32,
    pub selected_pet_id: Option<String>,
    pub window_placement_json: Option<JsonDocument>,
    pub reducer_json: Option<JsonDocument>,
    pub reducer_revision: i64,
    pub presentation_state: String,
    pub presentation_since_ms: i64,
    pub minimum_dwell_ms: i64,
}

#[derive(Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = sessions)]
#[diesel(primary_key(provider, session_id))]
#[diesel(check_for_backend(Sqlite))]
pub struct SessionRow {
    pub provider: String,
    pub session_id: String,
    pub current_turn_id: Option<String>,
    pub project_json: Option<JsonDocument>,
    pub updated_at_ms: i64,
    pub last_event_id: String,
    pub ended: i32,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = sessions)]
pub struct NewSession<'a> {
    pub provider: &'a str,
    pub session_id: &'a str,
    pub current_turn_id: Option<&'a str>,
    pub project_json: Option<&'a JsonDocument>,
    pub updated_at_ms: i64,
    pub last_event_id: &'a str,
    pub ended: i32,
}

#[derive(Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = turns)]
#[diesel(primary_key(provider, session_id, turn_id))]
#[diesel(check_for_backend(Sqlite))]
pub struct TurnRow {
    pub provider: String,
    pub session_id: String,
    pub turn_id: String,
    pub phase: String,
    pub updated_at_ms: i64,
    pub last_event_id: String,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = turns)]
pub struct NewTurn<'a> {
    pub provider: &'a str,
    pub session_id: &'a str,
    pub turn_id: &'a str,
    pub phase: &'a str,
    pub updated_at_ms: i64,
    pub last_event_id: &'a str,
}

#[derive(Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = notifications)]
#[diesel(check_for_backend(Sqlite))]
pub struct NotificationRow {
    pub id: String,
    pub provider: String,
    pub event_id: String,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub kind: String,
    pub state: String,
    pub occurred_at_ms: i64,
    pub project_json: Option<JsonDocument>,
    pub summary_json: Option<JsonDocument>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = notifications)]
pub struct NewNotification<'a> {
    pub id: &'a str,
    pub provider: &'a str,
    pub event_id: &'a str,
    pub session_id: &'a str,
    pub turn_id: Option<&'a str>,
    pub kind: &'a str,
    pub state: &'a str,
    pub occurred_at_ms: i64,
    pub project_json: Option<&'a JsonDocument>,
    pub summary_json: Option<&'a JsonDocument>,
}

#[derive(Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = recent_events)]
#[diesel(primary_key(provider, event_id))]
#[diesel(check_for_backend(Sqlite))]
pub struct RecentEventRow {
    pub provider: String,
    pub event_id: String,
    pub observed_at_ms: i64,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = recent_events)]
pub struct NewRecentEvent<'a> {
    pub provider: &'a str,
    pub event_id: &'a str,
    pub observed_at_ms: i64,
}

#[derive(Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = inbound_spool)]
#[diesel(primary_key(provider, event_id))]
#[diesel(check_for_backend(Sqlite))]
pub struct InboundSpoolRow {
    pub provider: String,
    pub event_id: String,
    pub payload_json: JsonDocument,
    pub priority: i32,
    pub occurred_at_ms: i64,
    pub inserted_at_ms: i64,
    pub status: String,
    pub claim_token: Option<String>,
    pub claimed_at_ms: Option<i64>,
    pub lease_expires_at_ms: Option<i64>,
    pub attempts: i32,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = inbound_spool)]
pub struct NewInboundSpool<'a> {
    pub provider: &'a str,
    pub event_id: &'a str,
    pub payload_json: &'a JsonDocument,
    pub priority: i32,
    pub occurred_at_ms: i64,
    pub inserted_at_ms: i64,
    pub status: &'a str,
    pub claim_token: Option<&'a str>,
    pub claimed_at_ms: Option<i64>,
    pub lease_expires_at_ms: Option<i64>,
    pub attempts: i32,
}

#[derive(Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = plugin_evidence)]
#[diesel(check_for_backend(Sqlite))]
pub struct PluginEvidenceRow {
    pub id: i32,
    pub evidence_json: JsonDocument,
    pub updated_at_ms: i64,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = plugin_evidence)]
pub struct NewPluginEvidence<'a> {
    pub id: i32,
    pub evidence_json: &'a JsonDocument,
    pub updated_at_ms: i64,
}

#[derive(Debug, Queryable, Selectable, Identifiable)]
#[diesel(table_name = lifecycle_events)]
#[diesel(primary_key(event_id))]
#[diesel(check_for_backend(Sqlite))]
pub struct LifecycleEventRow {
    pub event_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub event_type: String,
    pub source: String,
    pub occurred_at_ms: i64,
    pub previous_state: Option<String>,
    pub current_state: Option<String>,
    pub details_json: Option<JsonDocument>,
    pub error_json: Option<JsonDocument>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = lifecycle_events)]
pub struct NewLifecycleEvent<'a> {
    pub event_id: &'a str,
    pub entity_type: &'a str,
    pub entity_id: &'a str,
    pub event_type: &'a str,
    pub source: &'a str,
    pub occurred_at_ms: i64,
    pub previous_state: Option<&'a str>,
    pub current_state: Option<&'a str>,
    pub details_json: Option<&'a JsonDocument>,
    pub error_json: Option<&'a JsonDocument>,
}
