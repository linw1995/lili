use lili_storage::models::NewInboundSpool;
use lili_storage::repository::{
    claim_inbound_spool, delete_claimed_inbound_spool, delete_inbound_spool, find_inbound_spool,
    increment_spool_metrics, insert_inbound_spool, list_inbound_spool, load_app_state,
    recover_expired_inbound_spool_claims, release_claimed_inbound_spool,
};
use lili_storage::transaction::with_short_transaction;
use lili_storage::{ApplicationPaths, JsonDocument, open, open_with_busy_timeout};
use std::time::Duration;
use uuid::Uuid;

use crate::spool::event_priority;
use crate::{
    MAX_SPOOL_RECORD_BYTES, NormalizedSessionEvent, SpoolEnqueueOutcome, SpoolError, SpoolLimits,
    SpoolMetrics,
};

const CLAIM_LEASE_MS: i64 = 30_000;
const HOOK_BUSY_TIMEOUT: Duration = Duration::from_millis(700);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteSpoolStore {
    paths: ApplicationPaths,
    limits: SpoolLimits,
    busy_timeout: Option<Duration>,
}

impl SqliteSpoolStore {
    pub fn new(paths: ApplicationPaths, limits: SpoolLimits) -> Self {
        Self {
            paths,
            limits,
            busy_timeout: None,
        }
    }

    pub fn for_application(paths: ApplicationPaths) -> Self {
        Self::new(paths, SpoolLimits::default())
    }

    pub fn for_hook(paths: ApplicationPaths) -> Self {
        Self {
            paths,
            limits: SpoolLimits::default(),
            busy_timeout: Some(HOOK_BUSY_TIMEOUT),
        }
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
        let mut database = self.open_database().map_err(database_error)?;
        let new_record = NewInboundSpool {
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
        };
        let stored = with_short_transaction(database.connection(), |connection| {
            insert_inbound_spool(connection, &new_record)?;
            Ok(
                find_inbound_spool(connection, event.provider.as_str(), event.event_id.as_str())?
                    .is_some(),
            )
        })
        .map_err(database_query_error)?;
        let retained = if self.busy_timeout.is_some() && stored {
            // Keep the hook-critical transaction limited to the durable insert. The desktop
            // drain retries retention when a concurrent hook still owns the SQLite writer lock.
            let maintenance = with_short_transaction(database.connection(), |connection| {
                let delta = enforce_limits(connection, self.limits, enqueued_at_ms)?;
                record_retention_delta(connection, delta)
            });
            if maintenance.is_ok() {
                match find_inbound_spool(
                    database.connection(),
                    event.provider.as_str(),
                    event.event_id.as_str(),
                ) {
                    Ok(Some(_)) | Err(_) => true,
                    Ok(None) => false,
                }
            } else {
                true
            }
        } else if stored {
            with_short_transaction(database.connection(), |connection| {
                let delta = enforce_limits(connection, self.limits, enqueued_at_ms)?;
                record_retention_delta(connection, delta)?;
                Ok(find_inbound_spool(
                    connection,
                    event.provider.as_str(),
                    event.event_id.as_str(),
                )?
                .is_some())
            })
            .map_err(database_query_error)?
        } else {
            false
        };
        Ok(if retained {
            SpoolEnqueueOutcome::Stored
        } else {
            SpoolEnqueueOutcome::DroppedByLimit
        })
    }

    pub fn recover_claims(&self, now_ms: u64) -> Result<usize, SpoolError> {
        let mut database = self.open_database().map_err(database_error)?;
        recover_expired_inbound_spool_claims(
            database.connection(),
            i64::try_from(now_ms).unwrap_or(i64::MAX),
        )
        .map_err(database_query_error)
    }

    pub fn claim_next(&self, now_ms: u64) -> Result<Option<ClaimedSqliteSpoolRecord>, SpoolError> {
        validate_limits(self.limits)?;
        let now_ms = i64::try_from(now_ms).unwrap_or(i64::MAX);
        let mut database = self.open_database().map_err(database_error)?;
        recover_expired_inbound_spool_claims(database.connection(), now_ms)
            .map_err(database_query_error)?;
        with_short_transaction(database.connection(), |connection| {
            let delta = enforce_limits(connection, self.limits, now_ms)?;
            record_retention_delta(connection, delta)
        })
        .map_err(database_query_error)?;
        loop {
            let claim_token = Uuid::new_v4().to_string();
            let row =
                claim_inbound_spool(database.connection(), now_ms, CLAIM_LEASE_MS, &claim_token)
                    .map_err(database_query_error)?;
            let Some(row) = row else {
                return Ok(None);
            };
            let event =
                match serde_json::from_str::<NormalizedSessionEvent>(row.payload_json.as_str()) {
                    Ok(event) if event.validate().is_ok() => event,
                    Ok(_) | Err(_) => {
                        with_short_transaction(database.connection(), |connection| {
                            delete_claimed_inbound_spool(
                                connection,
                                &row.provider,
                                &row.event_id,
                                &claim_token,
                            )?;
                            increment_spool_metrics(connection, 0, 0, 1)
                        })
                        .map_err(database_query_error)?;
                        continue;
                    }
                };
            return Ok(Some(ClaimedSqliteSpoolRecord {
                store: self.clone(),
                provider: row.provider,
                event_id: row.event_id,
                claim_token,
                event,
                committed: false,
            }));
        }
    }

    pub fn metrics(&self) -> Result<SpoolMetrics, SpoolError> {
        let mut database = self.open_database().map_err(database_error)?;
        let _ = list_inbound_spool(database.connection()).map_err(database_query_error)?;
        let row = load_app_state(database.connection()).map_err(database_query_error)?;
        let mut metrics = SpoolMetrics::default();
        metrics.expired_drops = u64::try_from(row.spool_expired_drops).unwrap_or(u64::MAX);
        metrics.limit_drops = u64::try_from(row.spool_limit_drops).unwrap_or(u64::MAX);
        metrics.malformed_drops = u64::try_from(row.spool_malformed_drops).unwrap_or(u64::MAX);
        Ok(metrics)
    }

    fn open_database(&self) -> Result<lili_storage::EmbeddedDatabase, lili_storage::DatabaseError> {
        self.busy_timeout.map_or_else(
            || open(&self.paths),
            |busy_timeout| open_with_busy_timeout(&self.paths, busy_timeout),
        )
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
        let mut database = self.store.open_database().map_err(database_error)?;
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
        let mut database = self.store.open_database().map_err(database_error)?;
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RetentionDelta {
    expired_drops: i64,
    limit_drops: i64,
    malformed_drops: i64,
}

fn record_retention_delta(
    connection: &mut diesel::sqlite::SqliteConnection,
    delta: RetentionDelta,
) -> diesel::QueryResult<()> {
    if delta != RetentionDelta::default() {
        increment_spool_metrics(
            connection,
            delta.expired_drops,
            delta.limit_drops,
            delta.malformed_drops,
        )?;
    }
    Ok(())
}

fn enforce_limits(
    connection: &mut diesel::sqlite::SqliteConnection,
    limits: SpoolLimits,
    now_ms: i64,
) -> diesel::QueryResult<RetentionDelta> {
    let records = list_inbound_spool(connection)?;
    let max_age_ms = i64::try_from(limits.max_age_ms()).unwrap_or(i64::MAX);
    let mut delta = RetentionDelta::default();
    let mut retained = Vec::with_capacity(records.len());
    for record in records {
        if record.status == "pending" && now_ms.saturating_sub(record.inserted_at_ms) > max_age_ms {
            delete_inbound_spool(connection, &record.provider, &record.event_id)?;
            delta.expired_drops = delta.expired_drops.saturating_add(1);
        } else {
            retained.push(record);
        }
    }
    let mut total_count = retained.len();
    let mut total_bytes = retained
        .iter()
        .map(|record| record.payload_json.as_str().len() as u64)
        .sum::<u64>();
    let mut pending = retained
        .into_iter()
        .filter(|record| record.status == "pending")
        .collect::<Vec<_>>();
    pending.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.inserted_at_ms.cmp(&right.inserted_at_ms))
    });
    while total_count > limits.max_count() || total_bytes > limits.max_bytes() {
        let Some(record) = pending.first().cloned() else {
            break;
        };
        pending.remove(0);
        total_count = total_count.saturating_sub(1);
        total_bytes = total_bytes.saturating_sub(record.payload_json.as_str().len() as u64);
        delete_inbound_spool(connection, &record.provider, &record.event_id)?;
        delta.limit_drops = delta.limit_drops.saturating_add(1);
    }
    Ok(delta)
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
        assert_eq!(store.metrics().unwrap().limit_drops, 1);
        std::fs::remove_dir_all(paths.root()).unwrap();
    }

    #[test]
    fn retention_preserves_claimed_records_for_lease_recovery() {
        let paths = paths();
        let limits = SpoolLimits::new(1, 64 * 1024, 1_000);
        let store = SqliteSpoolStore::new(paths.clone(), limits);
        let claimed_event = event("claimed", "turn_completed");
        let pending_event = event("pending", "turn_completed");
        store.enqueue(&claimed_event, 1).unwrap();
        let claimed = store.claim_next(2).unwrap().unwrap();

        assert_eq!(
            store.enqueue(&pending_event, 3).unwrap(),
            SpoolEnqueueOutcome::DroppedByLimit
        );
        claimed.commit().unwrap();

        assert!(store.claim_next(4).unwrap().is_none());
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

    #[test]
    fn expired_pending_records_are_removed_before_claiming() {
        let paths = paths();
        let limits = SpoolLimits::new(8, 64 * 1024, 10);
        let store = SqliteSpoolStore::new(paths.clone(), limits);
        store
            .enqueue(&event("expired", "turn_completed"), 1)
            .unwrap();

        assert!(store.claim_next(12).unwrap().is_none());
        let mut database = open(&paths).unwrap();
        assert!(
            list_inbound_spool(database.connection())
                .unwrap()
                .is_empty()
        );
        drop(database);
        assert_eq!(store.metrics().unwrap().expired_drops, 1);
        std::fs::remove_dir_all(paths.root()).unwrap();
    }

    #[test]
    fn invalid_spool_records_are_removed_without_blocking_valid_records() {
        let paths = paths();
        let store = SqliteSpoolStore::for_application(paths.clone());
        let invalid_payload = JsonDocument::parse(r#"{"version":1}"#).unwrap();
        let mut database = open(&paths).unwrap();
        insert_inbound_spool(
            database.connection(),
            &NewInboundSpool {
                provider: "codex",
                event_id: "invalid",
                payload_json: &invalid_payload,
                priority: 2,
                occurred_at_ms: 1,
                inserted_at_ms: 1,
                status: "pending",
                claim_token: None,
                claimed_at_ms: None,
                lease_expires_at_ms: None,
                attempts: 0,
            },
        )
        .unwrap();
        drop(database);
        store.enqueue(&event("valid", "turn_completed"), 2).unwrap();

        let claimed = store.claim_next(3).unwrap().unwrap();
        assert_eq!(claimed.event().event_id.as_str(), "valid");
        claimed.commit().unwrap();

        let mut database = open(&paths).unwrap();
        assert!(
            find_inbound_spool(database.connection(), "codex", "invalid")
                .unwrap()
                .is_none()
        );
        drop(database);
        assert_eq!(store.metrics().unwrap().malformed_drops, 1);
        std::fs::remove_dir_all(paths.root()).unwrap();
    }

    #[test]
    fn concurrent_first_open_does_not_fail_migrations() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let paths = paths();
        let store = SqliteSpoolStore::for_application(paths.clone());
        let barrier = Arc::new(Barrier::new(5));
        let workers = (0..5)
            .map(|index| {
                let barrier = barrier.clone();
                let store = store.clone();
                thread::spawn(move || {
                    barrier.wait();
                    store.enqueue(&event(&format!("event-open-{index}"), "turn_completed"), 1)
                })
            })
            .collect::<Vec<_>>();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert!(results.iter().all(Result::is_ok), "{results:?}");
        std::fs::remove_dir_all(paths.root()).unwrap();
    }

    #[test]
    fn concurrent_direct_and_plugin_events_deduplicate_by_identity() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let paths = paths();
        let store = SqliteSpoolStore::for_application(paths.clone());
        let barrier = Arc::new(Barrier::new(4));
        let workers = [
            ("event-shared", "hook:Stop"),
            ("event-shared", "plugin:lili@lili-local:0.1.0:hook:Stop"),
            ("event-unique-a", "hook:SessionStart"),
            ("event-unique-b", "plugin:lili@lili-local:0.1.0:hook:Stop"),
        ]
        .map(|(event_id, source)| {
            let barrier = barrier.clone();
            let store = store.clone();
            thread::spawn(move || {
                let mut event = event(event_id, "turn_completed");
                event.source_discriminator = source.to_owned();
                barrier.wait();
                store.enqueue(&event, 1)
            })
        });
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert!(results.iter().all(Result::is_ok), "{results:?}");
        let mut database = open(&paths).unwrap();
        let records = list_inbound_spool(database.connection()).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(
            records
                .iter()
                .filter(|record| record.event_id == "event-shared")
                .count(),
            1
        );
        drop(database);
        std::fs::remove_dir_all(paths.root()).unwrap();
    }
}
