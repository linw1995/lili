use lili_storage::models::NewInboundSpool;
use lili_storage::repository::{
    claim_inbound_spool, delete_claimed_inbound_spool, delete_inbound_spool, find_inbound_spool,
    insert_inbound_spool, list_inbound_spool, recover_expired_inbound_spool_claims,
    release_claimed_inbound_spool,
};
use lili_storage::transaction::with_short_transaction;
use lili_storage::{ApplicationPaths, JsonDocument, open};
use uuid::Uuid;

use crate::spool::event_priority;
use crate::{
    MAX_SPOOL_RECORD_BYTES, NormalizedSessionEvent, SpoolEnqueueOutcome, SpoolError, SpoolLimits,
    SpoolMetrics,
};

const CLAIM_LEASE_MS: i64 = 30_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteSpoolStore {
    paths: ApplicationPaths,
    limits: SpoolLimits,
}

impl SqliteSpoolStore {
    pub fn new(paths: ApplicationPaths, limits: SpoolLimits) -> Self {
        Self { paths, limits }
    }

    pub fn for_application(paths: ApplicationPaths) -> Self {
        Self::new(paths, SpoolLimits::default())
    }

    pub fn database_path(&self) -> std::path::PathBuf {
        self.paths.database_path()
    }

    pub fn enqueue(
        &self,
        event: &NormalizedSessionEvent,
        enqueued_at_ms: u64,
    ) -> Result<SpoolEnqueueOutcome, SpoolError> {
        validate_limits(self.limits)?;
        event.validate().map_err(|_| SpoolError::InvalidEvent)?;
        let payload_json = JsonDocument::parse(
            serde_json::to_string(event).map_err(|_| SpoolError::MalformedRecord)?,
        )
        .map_err(|_| SpoolError::MalformedRecord)?;
        if payload_json.as_str().len() > MAX_SPOOL_RECORD_BYTES {
            return Err(SpoolError::RecordTooLarge);
        }
        let enqueued_at_ms = i64::try_from(enqueued_at_ms).unwrap_or(i64::MAX);
        let mut database = open(&self.paths).map_err(database_error)?;
        let retained = with_short_transaction(database.connection(), |connection| {
            insert_inbound_spool(
                connection,
                &NewInboundSpool {
                    provider: event.provider.as_str(),
                    event_id: event.event_id.as_str(),
                    payload_json: &payload_json,
                    priority: i32::from(event_priority(event.event_type)),
                    occurred_at_ms: i64::try_from(event.occurred_at_ms).unwrap_or(i64::MAX),
                    inserted_at_ms: enqueued_at_ms,
                    status: "pending",
                    claim_token: None,
                    claimed_at_ms: None,
                    lease_expires_at_ms: None,
                    attempts: 0,
                },
            )?;
            enforce_limits(connection, self.limits, enqueued_at_ms)?;
            Ok(
                find_inbound_spool(connection, event.provider.as_str(), event.event_id.as_str())?
                    .is_some(),
            )
        })
        .map_err(database_query_error)?;
        Ok(if retained {
            SpoolEnqueueOutcome::Stored
        } else {
            SpoolEnqueueOutcome::DroppedByLimit
        })
    }

    pub fn recover_claims(&self, now_ms: u64) -> Result<usize, SpoolError> {
        let mut database = open(&self.paths).map_err(database_error)?;
        recover_expired_inbound_spool_claims(
            database.connection(),
            i64::try_from(now_ms).unwrap_or(i64::MAX),
        )
        .map_err(database_query_error)
    }

    pub fn claim_next(&self, now_ms: u64) -> Result<Option<ClaimedSqliteSpoolRecord>, SpoolError> {
        validate_limits(self.limits)?;
        let now_ms = i64::try_from(now_ms).unwrap_or(i64::MAX);
        let mut database = open(&self.paths).map_err(database_error)?;
        recover_expired_inbound_spool_claims(database.connection(), now_ms)
            .map_err(database_query_error)?;
        let claim_token = Uuid::new_v4().to_string();
        let row = claim_inbound_spool(database.connection(), now_ms, CLAIM_LEASE_MS, &claim_token)
            .map_err(database_query_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let event = serde_json::from_str(row.payload_json.as_str())
            .map_err(|_| SpoolError::MalformedRecord)?;
        Ok(Some(ClaimedSqliteSpoolRecord {
            store: self.clone(),
            provider: row.provider,
            event_id: row.event_id,
            claim_token,
            event,
            committed: false,
        }))
    }

    pub fn metrics(&self) -> Result<SpoolMetrics, SpoolError> {
        let mut database = open(&self.paths).map_err(database_error)?;
        let _ = list_inbound_spool(database.connection()).map_err(database_query_error)?;
        Ok(SpoolMetrics::default())
    }
}

pub struct ClaimedSqliteSpoolRecord {
    store: SqliteSpoolStore,
    provider: String,
    event_id: String,
    claim_token: String,
    event: NormalizedSessionEvent,
    committed: bool,
}

impl ClaimedSqliteSpoolRecord {
    pub fn event(&self) -> &NormalizedSessionEvent {
        &self.event
    }

    pub fn commit(mut self) -> Result<(), SpoolError> {
        let mut database = open(&self.store.paths).map_err(database_error)?;
        let deleted = delete_claimed_inbound_spool(
            database.connection(),
            &self.provider,
            &self.event_id,
            &self.claim_token,
        )
        .map_err(database_query_error)?;
        if !deleted {
            return Err(SpoolError::MalformedRecord);
        }
        self.committed = true;
        Ok(())
    }

    pub fn release(mut self) -> Result<(), SpoolError> {
        self.release_inner()?;
        self.committed = true;
        Ok(())
    }

    fn release_inner(&self) -> Result<(), SpoolError> {
        let mut database = open(&self.store.paths).map_err(database_error)?;
        release_claimed_inbound_spool(
            database.connection(),
            &self.provider,
            &self.event_id,
            &self.claim_token,
        )
        .map_err(database_query_error)?;
        Ok(())
    }
}

impl Drop for ClaimedSqliteSpoolRecord {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.release_inner();
        }
    }
}

fn enforce_limits(
    connection: &mut diesel::sqlite::SqliteConnection,
    limits: SpoolLimits,
    now_ms: i64,
) -> diesel::QueryResult<()> {
    let records = list_inbound_spool(connection)?;
    let max_age_ms = i64::try_from(limits.max_age_ms()).unwrap_or(i64::MAX);
    let mut retained = Vec::with_capacity(records.len());
    for record in records {
        if now_ms.saturating_sub(record.inserted_at_ms) > max_age_ms {
            delete_inbound_spool(connection, &record.provider, &record.event_id)?;
        } else {
            retained.push(record);
        }
    }
    let mut records = retained;
    let mut total_bytes = records
        .iter()
        .map(|record| record.payload_json.as_str().len() as u64)
        .sum::<u64>();
    records.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.inserted_at_ms.cmp(&right.inserted_at_ms))
    });
    while records.len() > limits.max_count() || total_bytes > limits.max_bytes() {
        let record = records.remove(0);
        total_bytes = total_bytes.saturating_sub(record.payload_json.as_str().len() as u64);
        delete_inbound_spool(connection, &record.provider, &record.event_id)?;
    }
    Ok(())
}

fn validate_limits(limits: SpoolLimits) -> Result<(), SpoolError> {
    if limits.max_count() == 0 || limits.max_bytes() == 0 || limits.max_age_ms() == 0 {
        return Err(SpoolError::InvalidLimits);
    }
    Ok(())
}

fn database_error(error: lili_storage::DatabaseError) -> SpoolError {
    SpoolError::Database(error.to_string())
}

fn database_query_error(error: diesel::result::Error) -> SpoolError {
    SpoolError::Database(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ProviderCapabilitiesInputV1, ProviderInputV1, SessionEventKind, normalize_provider_input,
    };

    fn paths() -> ApplicationPaths {
        ApplicationPaths::from_root(
            std::env::temp_dir().join(format!("lili-sqlite-spool-{}", Uuid::new_v4())),
        )
        .unwrap()
    }

    fn event(id: &str, event_type: &str) -> NormalizedSessionEvent {
        normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some(event_type.to_owned()),
            event_id: Some(id.to_owned()),
            session_id: Some("session-1".to_owned()),
            turn_id: matches!(
                event_type,
                "turn_started" | "attention_required" | "turn_completed" | "turn_failed"
            )
            .then(|| "turn-1".to_owned()),
            occurred_at_ms: Some(1),
            project: None,
            summary: None,
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap()
    }

    #[test]
    fn enqueue_claim_and_commit_round_trip_through_sqlite() {
        let paths = paths();
        let store = SqliteSpoolStore::new(paths.clone(), SpoolLimits::default());
        let event = event("event-1", "turn_completed");
        assert_eq!(
            store.enqueue(&event, 1).unwrap(),
            SpoolEnqueueOutcome::Stored
        );
        let claimed = store.claim_next(2).unwrap().unwrap();
        assert_eq!(claimed.event().event_id.as_str(), "event-1");
        claimed.commit().unwrap();
        let mut database = open(&paths).unwrap();
        assert!(
            list_inbound_spool(database.connection())
                .unwrap()
                .is_empty()
        );
        drop(database);
        std::fs::remove_dir_all(paths.root()).unwrap();
    }

    #[test]
    fn retention_drops_lower_priority_records_first() {
        let paths = paths();
        let limits = SpoolLimits::new(1, 64 * 1024, 1_000);
        let store = SqliteSpoolStore::new(paths.clone(), limits);
        let completion = event("completion", "turn_completed");
        let attention = event("attention", "attention_required");
        assert_eq!(
            store.enqueue(&completion, 1).unwrap(),
            SpoolEnqueueOutcome::Stored
        );
        assert_eq!(
            store.enqueue(&attention, 2).unwrap(),
            SpoolEnqueueOutcome::Stored
        );
        let mut database = open(&paths).unwrap();
        assert!(
            find_inbound_spool(database.connection(), "codex", "completion")
                .unwrap()
                .is_none()
        );
        assert!(
            find_inbound_spool(database.connection(), "codex", "attention")
                .unwrap()
                .is_some()
        );
        drop(database);
        std::fs::remove_dir_all(paths.root()).unwrap();
    }

    #[test]
    fn expired_claim_is_recovered_by_the_next_claim() {
        let paths = paths();
        let store = SqliteSpoolStore::new(paths.clone(), SpoolLimits::default());
        let event = event("event-lease", "turn_completed");
        store.enqueue(&event, 1).unwrap();
        let first = store.claim_next(2).unwrap().unwrap();
        assert_eq!(first.event().event_type, SessionEventKind::TurnCompleted);
        std::mem::forget(first);
        let second = store
            .claim_next(2 + CLAIM_LEASE_MS as u64 + 1)
            .unwrap()
            .unwrap();
        assert_eq!(second.event().event_id.as_str(), "event-lease");
        second.commit().unwrap();
        std::fs::remove_dir_all(paths.root()).unwrap();
    }
}
