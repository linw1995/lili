#![cfg(unix)]

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use lili_lib::hook_forwarder::{
    HookOutcome, OFFLINE_FALLBACK_BUDGET, ONLINE_FORWARDING_BUDGET, UNRESPONSIVE_ENDPOINT_BUDGET,
    process_payload,
};
use lili_session::{
    BoundForwardingEndpoint, ForwardingAckDisposition, ForwardingVerifier, SqliteSpoolStore,
};
use lili_storage::ApplicationPaths;

const SAMPLE_COUNT: usize = 9;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "lili-hook-latency-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn payload(index: usize) -> Vec<u8> {
    format!(
        r#"{{"version":1,"provider":"codex","type":"turn_completed","eventId":"event-{index}","sessionId":"session-1","turnId":"turn-{index}","occurredAtMs":1000}}"#
    )
    .into_bytes()
}

fn percentile_95(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[((samples.len() * 95).div_ceil(100)).saturating_sub(1)]
}

#[tokio::test]
async fn online_forwarding_p95_stays_within_budget() {
    let temp = TempDir::new();
    let paths = ApplicationPaths::from_root(temp.0.clone()).unwrap();
    let runtime_dir = paths.runtime_root();
    let endpoint = BoundForwardingEndpoint::bind(&runtime_dir).unwrap();
    let mut verifier = ForwardingVerifier::new(endpoint.credentials());
    let server = tokio::spawn(async move {
        for _ in 0..SAMPLE_COUNT {
            let mut connection = endpoint.accept().await.unwrap();
            let payload = connection.read_payload().await.unwrap();
            let verified = verifier.verify_payload(&payload, 1_000).unwrap();
            connection
                .write_acknowledgement(
                    &verified.acknowledgement(ForwardingAckDisposition::Accepted),
                )
                .await
                .unwrap();
        }
    });

    let mut samples = Vec::with_capacity(SAMPLE_COUNT);
    for index in 0..SAMPLE_COUNT {
        let started = Instant::now();
        let result = process_payload(&paths, &payload(index), 1_000).await;
        samples.push(started.elapsed());
        assert_eq!(result.outcome, Some(HookOutcome::Delivered));
    }
    server.await.unwrap();
    let p95 = percentile_95(samples);
    eprintln!("online_forwarding_p95_ms={}", p95.as_millis());
    assert!(
        p95 <= ONLINE_FORWARDING_BUDGET,
        "online forwarding exceeded its p95 latency budget"
    );
}

#[tokio::test]
async fn offline_fallback_p95_stays_within_budget() {
    let temp = TempDir::new();
    let paths = ApplicationPaths::from_root(temp.0.clone()).unwrap();
    let mut samples = Vec::with_capacity(SAMPLE_COUNT);
    for index in 0..SAMPLE_COUNT {
        let started = Instant::now();
        let result = process_payload(&paths, &payload(index), 1_000).await;
        samples.push(started.elapsed());
        assert_eq!(result.outcome, Some(HookOutcome::Spooled));
    }
    let p95 = percentile_95(samples);
    eprintln!("offline_fallback_p95_ms={}", p95.as_millis());
    assert!(
        p95 <= OFFLINE_FALLBACK_BUDGET,
        "offline fallback exceeded its p95 latency budget"
    );
}

#[tokio::test]
async fn unresponsive_native_endpoint_falls_back_without_renderer_dependency() {
    let temp = TempDir::new();
    let paths = ApplicationPaths::from_root(temp.0.clone()).unwrap();
    let runtime_dir = paths.runtime_root();
    let endpoint = BoundForwardingEndpoint::bind(&runtime_dir).unwrap();
    let server = tokio::spawn(async move {
        let mut connection = endpoint.accept().await.unwrap();
        connection.read_payload().await.unwrap();
        std::future::pending::<()>().await;
    });

    let started = Instant::now();
    let result = process_payload(&paths, &payload(0), 1_000).await;
    let elapsed = started.elapsed();
    eprintln!("unresponsive_endpoint_fallback_ms={}", elapsed.as_millis());
    server.abort();
    assert_eq!(result.outcome, Some(HookOutcome::Spooled));
    assert!(
        elapsed <= UNRESPONSIVE_ENDPOINT_BUDGET,
        "unresponsive endpoint fallback exceeded its latency budget"
    );
    assert!(
        SqliteSpoolStore::for_application(paths)
            .claim_next(1_001)
            .unwrap()
            .is_some()
    );
}
