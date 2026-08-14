use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{NormalizedSessionEvent, SessionEventKind};

const SPOOL_VERSION: u16 = 1;
const METRICS_FILE_NAME: &str = "metrics.json";
const LOCK_DIRECTORY_NAME: &str = ".lock";
pub const MAX_SPOOL_RECORD_BYTES: usize = 64 * 1024;
const MAX_METRICS_BYTES: u64 = 4 * 1024;
const LOCK_RETRY_COUNT: usize = 250;
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(2);
const STALE_LOCK_AGE: Duration = Duration::from_secs(30);
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpoolLimits {
    max_count: usize,
    max_bytes: u64,
    max_age_ms: u64,
}

impl SpoolLimits {
    pub const fn new(max_count: usize, max_bytes: u64, max_age_ms: u64) -> Self {
        Self {
            max_count,
            max_bytes,
            max_age_ms,
        }
    }
}

impl Default for SpoolLimits {
    fn default() -> Self {
        Self::new(256, 4 * 1024 * 1024, 24 * 60 * 60 * 1_000)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpoolMetrics {
    #[serde(default = "spool_version")]
    version: u16,
    pub expired_drops: u64,
    pub limit_drops: u64,
    pub malformed_drops: u64,
}

const fn spool_version() -> u16 {
    SPOOL_VERSION
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpoolEnqueueOutcome {
    Stored,
    DroppedByLimit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpoolStore {
    directory: PathBuf,
    limits: SpoolLimits,
}

impl SpoolStore {
    pub fn new(directory: impl Into<PathBuf>, limits: SpoolLimits) -> Self {
        Self {
            directory: directory.into(),
            limits,
        }
    }

    pub fn for_codex_home(codex_home: &Path) -> Self {
        Self::new(
            codex_home.join("lili").join("spool"),
            SpoolLimits::default(),
        )
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn enqueue(
        &self,
        event: &NormalizedSessionEvent,
        enqueued_at_ms: u64,
    ) -> Result<SpoolEnqueueOutcome, SpoolError> {
        validate_limits(self.limits)?;
        event.validate().map_err(|_| SpoolError::InvalidEvent)?;
        ensure_private_directory(&self.directory)?;
        let _lock = SpoolLock::acquire(&self.directory)?;
        let mut metrics = self.load_metrics_unlocked()?;
        let original_metrics = metrics.clone();
        let record = SpoolRecord {
            version: SPOOL_VERSION,
            enqueued_at_ms,
            event: event.clone(),
        };
        let payload = serde_json::to_vec(&record).map_err(|_| SpoolError::MalformedRecord)?;
        if payload.len() > MAX_SPOOL_RECORD_BYTES {
            return Err(SpoolError::RecordTooLarge);
        }
        let pending_path = self.directory.join(pending_file_name(enqueued_at_ms)?);
        atomic_write(&pending_path, &payload, MAX_SPOOL_RECORD_BYTES as u64)?;
        let candidates = self.collect_pending(enqueued_at_ms, &mut metrics)?;
        let retained = self.enforce_limits(candidates, &mut metrics)?;
        self.save_metrics_if_changed(&original_metrics, &metrics)?;
        if retained
            .iter()
            .any(|candidate| candidate.path == pending_path)
        {
            Ok(SpoolEnqueueOutcome::Stored)
        } else {
            Ok(SpoolEnqueueOutcome::DroppedByLimit)
        }
    }

    pub fn recover_claims(&self) -> Result<usize, SpoolError> {
        ensure_private_directory(&self.directory)?;
        let _lock = SpoolLock::acquire(&self.directory)?;
        let mut recovered = 0;
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".write-") && name.ends_with(".tmp") {
                remove_unsafe_record(&entry.path())?;
                continue;
            }
            if !name.contains(".claim-") {
                continue;
            }
            validate_record_file(&entry.path())?;
            let target = self.directory.join(pending_file_name_from_claim(&name)?);
            let target = if target.exists() {
                self.directory.join(pending_file_name(now_nonce_seed())?)
            } else {
                target
            };
            fs::rename(entry.path(), target)?;
            recovered += 1;
        }
        if recovered > 0 {
            sync_directory(&self.directory)?;
        }
        Ok(recovered)
    }

    pub fn claim_next(&self, now_ms: u64) -> Result<Option<ClaimedSpoolRecord>, SpoolError> {
        validate_limits(self.limits)?;
        ensure_private_directory(&self.directory)?;
        let _lock = SpoolLock::acquire(&self.directory)?;
        let mut metrics = self.load_metrics_unlocked()?;
        let original_metrics = metrics.clone();
        let mut candidates = self.collect_pending(now_ms, &mut metrics)?;
        candidates.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.record.enqueued_at_ms.cmp(&right.record.enqueued_at_ms))
                .then_with(|| left.path.cmp(&right.path))
        });
        self.save_metrics_if_changed(&original_metrics, &metrics)?;
        let Some(candidate) = candidates.into_iter().next() else {
            return Ok(None);
        };
        let sequence = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let base = candidate
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".pending"))
            .ok_or(SpoolError::MalformedRecord)?;
        let claimed_path = self
            .directory
            .join(format!("{base}.claim-{}-{sequence}", std::process::id()));
        fs::rename(&candidate.path, &claimed_path)?;
        sync_directory(&self.directory)?;
        Ok(Some(ClaimedSpoolRecord {
            store: self.clone(),
            pending_path: candidate.path,
            claimed_path,
            record: candidate.record,
            committed: false,
        }))
    }

    pub fn metrics(&self) -> Result<SpoolMetrics, SpoolError> {
        ensure_private_directory(&self.directory)?;
        let _lock = SpoolLock::acquire(&self.directory)?;
        self.load_metrics_unlocked()
    }

    fn collect_pending(
        &self,
        now_ms: u64,
        metrics: &mut SpoolMetrics,
    ) -> Result<Vec<PendingCandidate>, SpoolError> {
        let mut candidates = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".pending") {
                continue;
            }
            let path = entry.path();
            match read_record(&path) {
                Ok((record, size)) => {
                    if now_ms.saturating_sub(record.enqueued_at_ms) > self.limits.max_age_ms {
                        fs::remove_file(path)?;
                        metrics.expired_drops = metrics.expired_drops.saturating_add(1);
                    } else {
                        candidates.push(PendingCandidate {
                            priority: event_priority(record.event.event_type),
                            path,
                            record,
                            size,
                        });
                    }
                }
                Err(SpoolError::Io(error)) => return Err(SpoolError::Io(error)),
                Err(_) => {
                    remove_unsafe_record(&path)?;
                    metrics.malformed_drops = metrics.malformed_drops.saturating_add(1);
                }
            }
        }
        Ok(candidates)
    }

    fn enforce_limits(
        &self,
        mut candidates: Vec<PendingCandidate>,
        metrics: &mut SpoolMetrics,
    ) -> Result<Vec<PendingCandidate>, SpoolError> {
        let mut total_bytes = candidates
            .iter()
            .map(|candidate| candidate.size)
            .sum::<u64>();
        candidates.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left.record.enqueued_at_ms.cmp(&right.record.enqueued_at_ms))
                .then_with(|| left.path.cmp(&right.path))
        });
        while candidates.len() > self.limits.max_count || total_bytes > self.limits.max_bytes {
            let candidate = candidates.remove(0);
            total_bytes = total_bytes.saturating_sub(candidate.size);
            fs::remove_file(candidate.path)?;
            metrics.limit_drops = metrics.limit_drops.saturating_add(1);
        }
        Ok(candidates)
    }

    fn load_metrics_unlocked(&self) -> Result<SpoolMetrics, SpoolError> {
        let path = self.directory.join(METRICS_FILE_NAME);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SpoolMetrics {
                    version: SPOOL_VERSION,
                    ..SpoolMetrics::default()
                });
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(SpoolError::UnsafePath);
        }
        validate_private_metadata(&metadata)?;
        if metadata.len() > MAX_METRICS_BYTES {
            return Err(SpoolError::MalformedMetrics);
        }
        let mut payload = Vec::with_capacity(metadata.len() as usize);
        File::open(path)?.read_to_end(&mut payload)?;
        let metrics: SpoolMetrics =
            serde_json::from_slice(&payload).map_err(|_| SpoolError::MalformedMetrics)?;
        if metrics.version != SPOOL_VERSION {
            return Err(SpoolError::UnsupportedVersion(metrics.version));
        }
        Ok(metrics)
    }

    fn save_metrics_unlocked(&self, metrics: &SpoolMetrics) -> Result<(), SpoolError> {
        let payload = serde_json::to_vec(metrics).map_err(|_| SpoolError::MalformedMetrics)?;
        atomic_write(
            &self.directory.join(METRICS_FILE_NAME),
            &payload,
            MAX_METRICS_BYTES,
        )
    }

    fn save_metrics_if_changed(
        &self,
        original: &SpoolMetrics,
        current: &SpoolMetrics,
    ) -> Result<(), SpoolError> {
        if current != original {
            self.save_metrics_unlocked(current)?;
        }
        Ok(())
    }
}

pub struct ClaimedSpoolRecord {
    store: SpoolStore,
    pending_path: PathBuf,
    claimed_path: PathBuf,
    record: SpoolRecord,
    committed: bool,
}

impl ClaimedSpoolRecord {
    pub fn event(&self) -> &NormalizedSessionEvent {
        &self.record.event
    }

    pub fn commit(mut self) -> Result<(), SpoolError> {
        fs::remove_file(&self.claimed_path)?;
        sync_directory(&self.store.directory)?;
        self.committed = true;
        Ok(())
    }

    pub fn release(mut self) -> Result<(), SpoolError> {
        self.release_inner()?;
        self.committed = true;
        Ok(())
    }

    fn release_inner(&self) -> Result<(), SpoolError> {
        let _lock = SpoolLock::acquire(&self.store.directory)?;
        if self.claimed_path.exists() {
            let target = if self.pending_path.exists() {
                self.store
                    .directory
                    .join(pending_file_name(now_nonce_seed())?)
            } else {
                self.pending_path.clone()
            };
            fs::rename(&self.claimed_path, target)?;
            sync_directory(&self.store.directory)?;
        }
        Ok(())
    }
}

impl Drop for ClaimedSpoolRecord {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.release_inner();
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpoolRecord {
    version: u16,
    enqueued_at_ms: u64,
    event: NormalizedSessionEvent,
}

struct PendingCandidate {
    priority: u8,
    path: PathBuf,
    record: SpoolRecord,
    size: u64,
}

struct SpoolLock {
    path: PathBuf,
}

impl SpoolLock {
    fn acquire(directory: &Path) -> Result<Self, SpoolError> {
        let path = directory.join(LOCK_DIRECTORY_NAME);
        for _ in 0..LOCK_RETRY_COUNT {
            match create_lock_directory(&path) {
                Ok(()) => {
                    configure_private_directory(&path, &fs::symlink_metadata(&path)?)?;
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let metadata = match fs::symlink_metadata(&path) {
                        Ok(metadata) => metadata,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => return Err(error.into()),
                    };
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(SpoolError::UnsafePath);
                    }
                    validate_private_metadata(&metadata)?;
                    let stale = metadata
                        .modified()
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > STALE_LOCK_AGE);
                    if stale {
                        fs::remove_dir(&path)?;
                        continue;
                    }
                    thread::sleep(LOCK_RETRY_DELAY);
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(SpoolError::Busy)
    }
}

#[cfg(unix)]
fn create_lock_directory(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_lock_directory(path: &Path) -> Result<(), std::io::Error> {
    fs::create_dir(path)
}

impl Drop for SpoolLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}

fn validate_limits(limits: SpoolLimits) -> Result<(), SpoolError> {
    if limits.max_count == 0 || limits.max_bytes == 0 || limits.max_age_ms == 0 {
        return Err(SpoolError::InvalidLimits);
    }
    Ok(())
}

fn read_record(path: &Path) -> Result<(SpoolRecord, u64), SpoolError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SpoolError::UnsafePath);
    }
    validate_private_metadata(&metadata)?;
    if metadata.len() > MAX_SPOOL_RECORD_BYTES as u64 {
        return Err(SpoolError::RecordTooLarge);
    }
    let mut payload = Vec::with_capacity(metadata.len() as usize);
    File::open(path)?
        .take(MAX_SPOOL_RECORD_BYTES as u64 + 1)
        .read_to_end(&mut payload)?;
    let record = decode_spool_record_inner(&payload)?;
    Ok((record, metadata.len()))
}

pub fn decode_spool_record(payload: &[u8]) -> Result<(u64, NormalizedSessionEvent), SpoolError> {
    let record = decode_spool_record_inner(payload)?;
    Ok((record.enqueued_at_ms, record.event))
}

fn decode_spool_record_inner(payload: &[u8]) -> Result<SpoolRecord, SpoolError> {
    if payload.len() > MAX_SPOOL_RECORD_BYTES {
        return Err(SpoolError::RecordTooLarge);
    }
    let record: SpoolRecord =
        serde_json::from_slice(payload).map_err(|_| SpoolError::MalformedRecord)?;
    if record.version != SPOOL_VERSION {
        return Err(SpoolError::UnsupportedVersion(record.version));
    }
    record
        .event
        .validate()
        .map_err(|_| SpoolError::InvalidEvent)?;
    Ok(record)
}

fn validate_record_file(path: &Path) -> Result<(), SpoolError> {
    read_record(path).map(|_| ())
}

fn remove_unsafe_record(path: &Path) -> Result<(), SpoolError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path)?;
        Ok(())
    } else {
        Err(SpoolError::UnsafePath)
    }
}

fn event_priority(event_type: SessionEventKind) -> u8 {
    match event_type {
        SessionEventKind::AttentionRequired => 3,
        SessionEventKind::TurnFailed => 2,
        SessionEventKind::SessionStarted
        | SessionEventKind::TurnStarted
        | SessionEventKind::AttentionResolved => 1,
        SessionEventKind::TurnCompleted | SessionEventKind::SessionEnded => 0,
    }
}

fn pending_file_name(enqueued_at_ms: u64) -> Result<String, SpoolError> {
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| SpoolError::Randomness)?;
    Ok(format!(
        "{enqueued_at_ms:020}-{}.pending",
        encode_hex(&nonce)
    ))
}

fn pending_file_name_from_claim(name: &str) -> Result<String, SpoolError> {
    let base = name
        .split_once(".claim-")
        .map(|(base, _)| base)
        .filter(|base| !base.is_empty())
        .ok_or(SpoolError::MalformedRecord)?;
    Ok(format!("{base}.pending"))
}

fn now_nonce_seed() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn atomic_write(path: &Path, payload: &[u8], limit: u64) -> Result<(), SpoolError> {
    if payload.len() as u64 > limit {
        return Err(SpoolError::RecordTooLarge);
    }
    let directory = path.parent().ok_or(SpoolError::UnsafePath)?;
    let sequence = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(".write-{}-{sequence}.tmp", std::process::id()));
    let mut guard = TemporaryFileGuard::new(temporary.clone());
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(payload)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    guard.commit();
    sync_directory(directory)?;
    Ok(())
}

struct TemporaryFileGuard {
    path: PathBuf,
    committed: bool,
}

impl TemporaryFileGuard {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TemporaryFileGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn ensure_private_directory(directory: &Path) -> Result<(), SpoolError> {
    fs::create_dir_all(directory)?;
    let metadata = fs::symlink_metadata(directory)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SpoolError::UnsafePath);
    }
    configure_private_directory(directory, &metadata)
}

#[cfg(unix)]
fn configure_private_directory(path: &Path, metadata: &fs::Metadata) -> Result<(), SpoolError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    if metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(SpoolError::WrongOwner);
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn configure_private_directory(_path: &Path, _metadata: &fs::Metadata) -> Result<(), SpoolError> {
    Ok(())
}

#[cfg(unix)]
fn validate_private_metadata(metadata: &fs::Metadata) -> Result<(), SpoolError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    if metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(SpoolError::WrongOwner);
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_metadata(_metadata: &fs::Metadata) -> Result<(), SpoolError> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(directory: &Path) -> Result<(), std::io::Error> {
    File::open(directory)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

#[derive(Debug, Error)]
pub enum SpoolError {
    #[error("spool I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("spool path is unsafe")]
    UnsafePath,
    #[error("spool object belongs to another user")]
    WrongOwner,
    #[error("spool is busy")]
    Busy,
    #[error("spool limits are invalid")]
    InvalidLimits,
    #[error("spool record exceeds 64 KiB")]
    RecordTooLarge,
    #[error("spool record is malformed")]
    MalformedRecord,
    #[error("spool event violates normalized invariants")]
    InvalidEvent,
    #[error("spool metrics are malformed")]
    MalformedMetrics,
    #[error("spool version {0} is unsupported")]
    UnsupportedVersion(u16),
    #[error("secure randomness is unavailable")]
    Randomness,
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use crate::{ProviderCapabilitiesInputV1, ProviderInputV1, normalize_provider_input};

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("lili-spool-{}-{sequence}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn event(id: &str, event_type: &str) -> NormalizedSessionEvent {
        normalize_provider_input(ProviderInputV1 {
            version: 1,
            provider: Some("codex".to_owned()),
            event_type: Some(event_type.to_owned()),
            event_id: Some(id.to_owned()),
            session_id: Some("session-1".to_owned()),
            turn_id: (!event_type.starts_with("session_")).then(|| "turn-1".to_owned()),
            occurred_at_ms: Some(10),
            project: None,
            summary: None,
            capabilities: ProviderCapabilitiesInputV1::default(),
            source_discriminator: None,
        })
        .unwrap()
    }

    #[test]
    fn failure_injection_during_spool_claim_releases_uncommitted_record() {
        let temp = TempDir::new();
        let store = SpoolStore::new(&temp.0, SpoolLimits::default());
        store
            .enqueue(&event("event-1", "turn_completed"), 100)
            .unwrap();
        let claim = store.claim_next(101).unwrap().unwrap();
        assert_eq!(claim.event().event_id.as_str(), "event-1");
        drop(claim);
        let claim = store.claim_next(102).unwrap().unwrap();
        claim.commit().unwrap();
        assert!(store.claim_next(103).unwrap().is_none());
    }

    #[test]
    fn recovery_restores_interrupted_claim() {
        let temp = TempDir::new();
        let store = SpoolStore::new(&temp.0, SpoolLimits::default());
        store
            .enqueue(&event("event-1", "turn_completed"), 100)
            .unwrap();
        let mut pending = fs::read_dir(&temp.0)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.to_string_lossy().ends_with(".pending"))
            .unwrap();
        let claim = pending.with_extension("claim-crashed");
        fs::rename(&pending, &claim).unwrap();
        pending.clear();
        assert_eq!(store.recover_claims().unwrap(), 1);
        assert!(store.claim_next(101).unwrap().is_some());
    }

    #[test]
    fn eviction_preserves_attention_before_terminal_events() {
        let temp = TempDir::new();
        let store = SpoolStore::new(&temp.0, SpoolLimits::new(2, 1024 * 1024, 10_000));
        store
            .enqueue(&event("completion-1", "turn_completed"), 100)
            .unwrap();
        store
            .enqueue(&event("attention", "attention_required"), 101)
            .unwrap();
        store
            .enqueue(&event("completion-2", "turn_completed"), 102)
            .unwrap();

        let first = store.claim_next(103).unwrap().unwrap();
        assert_eq!(first.event().event_id.as_str(), "attention");
        first.commit().unwrap();
        let second = store.claim_next(103).unwrap().unwrap();
        assert_eq!(second.event().event_id.as_str(), "completion-2");
        second.commit().unwrap();
        assert_eq!(store.metrics().unwrap().limit_drops, 1);
    }

    #[test]
    fn expired_records_are_dropped_with_aggregate_metrics() {
        let temp = TempDir::new();
        let store = SpoolStore::new(&temp.0, SpoolLimits::new(4, 1024 * 1024, 10));
        store
            .enqueue(&event("event-1", "turn_completed"), 100)
            .unwrap();
        assert!(store.claim_next(111).unwrap().is_none());
        assert_eq!(store.metrics().unwrap().expired_drops, 1);
    }

    #[test]
    fn symlinked_spool_directory_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new();
        let target = temp.0.join("target");
        fs::create_dir(&target).unwrap();
        let linked = temp.0.join("linked");
        symlink(&target, &linked).unwrap();
        let store = SpoolStore::new(linked, SpoolLimits::default());
        assert!(matches!(
            store.enqueue(&event("event-1", "turn_completed"), 100),
            Err(SpoolError::UnsafePath)
        ));
        assert_eq!(fs::read_dir(target).unwrap().count(), 0);
    }

    #[test]
    fn interrupted_temporary_write_is_removed_during_recovery() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new();
        let store = SpoolStore::new(&temp.0, SpoolLimits::default());
        store
            .enqueue(&event("event-1", "turn_completed"), 100)
            .unwrap();
        let temporary = temp.0.join(".write-crashed.tmp");
        fs::write(&temporary, b"partial").unwrap();
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600)).unwrap();
        store.recover_claims().unwrap();
        assert!(!temporary.exists());
    }

    #[test]
    fn concurrent_forwarders_preserve_bounds_and_valid_records() {
        use std::sync::{Arc, Barrier};

        let temp = TempDir::new();
        let store = Arc::new(SpoolStore::new(
            &temp.0,
            SpoolLimits::new(8, 1024 * 1024, 10_000),
        ));
        let barrier = Arc::new(Barrier::new(8));
        let threads = (0..8)
            .map(|index| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    store
                        .enqueue(
                            &event(&format!("event-{index}"), "turn_completed"),
                            100 + index,
                        )
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            assert_eq!(thread.join().unwrap(), SpoolEnqueueOutcome::Stored);
        }

        let mut ids = std::collections::BTreeSet::new();
        while let Some(claim) = store.claim_next(200).unwrap() {
            ids.insert(claim.event().event_id.as_str().to_owned());
            claim.commit().unwrap();
        }
        assert_eq!(ids.len(), 8);
    }
}
