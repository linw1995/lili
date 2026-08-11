use std::{ffi::OsString, io::Read, path::Path, time::Duration};

use lili_integration::LILI_INTEGRATION_ID;
use lili_pet::resolve_codex_home;
use lili_session::{
    ForwardingCredentialStore, MAX_PROVIDER_PAYLOAD_BYTES, SpoolEnqueueOutcome, SpoolStore,
    deliver_forwarding_message, normalize_hook_json,
};

pub const CONNECTION_DEADLINE: Duration = Duration::from_millis(150);
pub const ONLINE_FORWARDING_BUDGET: Duration = Duration::from_millis(250);
pub const OFFLINE_FALLBACK_BUDGET: Duration = Duration::from_millis(750);
pub const UNRESPONSIVE_ENDPOINT_BUDGET: Duration = Duration::from_millis(750);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum HookExitCode {
    Success = 0,
    Usage = 2,
    InvalidInput = 3,
    DeliveryFailed = 4,
}

impl HookExitCode {
    pub const fn value(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HookOutcome {
    Delivered,
    Spooled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HookResult {
    pub exit_code: HookExitCode,
    pub outcome: Option<HookOutcome>,
    pub diagnostic: Option<&'static str>,
}

impl HookResult {
    fn success(outcome: HookOutcome) -> Self {
        Self {
            exit_code: HookExitCode::Success,
            outcome: Some(outcome),
            diagnostic: None,
        }
    }

    fn failure(exit_code: HookExitCode, diagnostic: &'static str) -> Self {
        Self {
            exit_code,
            outcome: None,
            diagnostic: Some(diagnostic),
        }
    }
}

pub async fn run_from_environment() -> HookResult {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let mut stdin = std::io::stdin().lock();
    let payload = match read_hook_payload(&arguments, &mut stdin) {
        Ok(payload) => payload,
        Err(result) => return result,
    };
    let codex_home = match resolve_codex_home() {
        Ok(codex_home) => codex_home,
        Err(_) => {
            return HookResult::failure(
                HookExitCode::DeliveryFailed,
                "Codex home could not be resolved",
            );
        }
    };
    process_payload(&codex_home, &payload, unix_time_ms()).await
}

pub async fn process_payload(codex_home: &Path, payload: &[u8], now_ms: u64) -> HookResult {
    let event = match normalize_hook_json(payload, now_ms) {
        Ok(event) => event,
        Err(_) => {
            return HookResult::failure(
                HookExitCode::InvalidInput,
                "hook payload is invalid or unsupported",
            );
        }
    };

    let credential_store =
        ForwardingCredentialStore::for_runtime_dir(&codex_home.join("lili").join("runtime"));
    let online = credential_store.load().ok().and_then(|record| {
        record
            .credentials()
            .ok()
            .and_then(|credentials| credentials.sign(event.clone(), now_ms).ok())
            .map(|message| (record, message))
    });
    if let Some((record, message)) = online
        && tokio::time::timeout(
            CONNECTION_DEADLINE,
            deliver_forwarding_message(&record, &message),
        )
        .await
        .is_ok_and(|result| result.is_ok())
    {
        return HookResult::success(HookOutcome::Delivered);
    }

    match SpoolStore::for_codex_home(codex_home).enqueue(&event, now_ms) {
        Ok(SpoolEnqueueOutcome::Stored) => HookResult::success(HookOutcome::Spooled),
        Ok(SpoolEnqueueOutcome::DroppedByLimit) => HookResult::failure(
            HookExitCode::DeliveryFailed,
            "hook event was dropped by the offline spool limit",
        ),
        Err(_) => HookResult::failure(
            HookExitCode::DeliveryFailed,
            "hook event could not be delivered or spooled",
        ),
    }
}

fn read_hook_payload<R: Read>(
    arguments: &[OsString],
    stdin: &mut R,
) -> Result<Vec<u8>, HookResult> {
    let arguments = match arguments {
        [flag, integration_id, rest @ ..] if flag == "--integration-id" => {
            if integration_id != LILI_INTEGRATION_ID {
                return Err(HookResult::failure(
                    HookExitCode::Usage,
                    "unknown integration identity",
                ));
            }
            rest
        }
        _ => arguments,
    };
    match arguments {
        [mode, payload] if mode == "--json-argv" => {
            let payload = payload.to_str().ok_or_else(|| {
                HookResult::failure(HookExitCode::InvalidInput, "hook argv must be valid UTF-8")
            })?;
            if payload.len() > MAX_PROVIDER_PAYLOAD_BYTES {
                return Err(HookResult::failure(
                    HookExitCode::InvalidInput,
                    "hook payload exceeds 64 KiB",
                ));
            }
            Ok(payload.as_bytes().to_vec())
        }
        [mode] if mode == "--json-stdin" => {
            let mut payload = Vec::new();
            stdin
                .take(MAX_PROVIDER_PAYLOAD_BYTES as u64 + 1)
                .read_to_end(&mut payload)
                .map_err(|_| {
                    HookResult::failure(HookExitCode::InvalidInput, "hook stdin could not be read")
                })?;
            if payload.len() > MAX_PROVIDER_PAYLOAD_BYTES {
                return Err(HookResult::failure(
                    HookExitCode::InvalidInput,
                    "hook payload exceeds 64 KiB",
                ));
            }
            Ok(payload)
        }
        _ => Err(HookResult::failure(
            HookExitCode::Usage,
            "usage: lili-hook --json-argv <json> | --json-stdin",
        )),
    }
}

fn unix_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::Cursor,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "lili-hook-forwarder-{}-{sequence}",
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

    fn payload() -> Vec<u8> {
        br#"{"version":1,"provider":"codex","type":"turn_completed","eventId":"event-1","sessionId":"session-1","turnId":"turn-1","occurredAtMs":10}"#.to_vec()
    }

    #[test]
    fn argv_and_stdin_modes_read_the_same_bounded_payload() {
        let payload = String::from_utf8(payload()).unwrap();
        let argv = read_hook_payload(
            &[OsString::from("--json-argv"), OsString::from(&payload)],
            &mut Cursor::new(Vec::new()),
        )
        .unwrap();
        let stdin = read_hook_payload(
            &[OsString::from("--json-stdin")],
            &mut Cursor::new(payload.as_bytes()),
        )
        .unwrap();
        assert_eq!(argv, stdin);
    }

    #[test]
    fn input_modes_reject_oversized_or_ambiguous_arguments() {
        let oversized = OsString::from("x".repeat(MAX_PROVIDER_PAYLOAD_BYTES + 1));
        assert_eq!(
            read_hook_payload(
                &[OsString::from("--json-argv"), oversized],
                &mut Cursor::new(Vec::new())
            )
            .unwrap_err()
            .exit_code,
            HookExitCode::InvalidInput
        );
        assert_eq!(
            read_hook_payload(&[], &mut Cursor::new(Vec::new()))
                .unwrap_err()
                .exit_code,
            HookExitCode::Usage
        );
    }

    #[tokio::test]
    async fn unavailable_endpoint_falls_back_to_one_normalized_spool_record() {
        let temp = TempDir::new();
        let result = process_payload(&temp.0, &payload(), 1_000).await;
        assert_eq!(result, HookResult::success(HookOutcome::Spooled));
        let spool = SpoolStore::for_codex_home(&temp.0);
        let claim = spool.claim_next(1_001).unwrap().unwrap();
        assert_eq!(claim.event().event_id.as_str(), "event-1");
        claim.commit().unwrap();
    }

    #[tokio::test]
    async fn malformed_payload_has_a_deterministic_exit_code_without_spooling() {
        let temp = TempDir::new();
        let result = process_payload(&temp.0, b"not-json", 1_000).await;
        assert_eq!(result.exit_code, HookExitCode::InvalidInput);
        assert!(!SpoolStore::for_codex_home(&temp.0).directory().exists());
    }

    #[test]
    fn installed_marker_is_accepted_but_unknown_identity_is_rejected() {
        let payload = String::from_utf8(payload()).unwrap();
        let marked = read_hook_payload(
            &[
                OsString::from("--integration-id"),
                OsString::from(LILI_INTEGRATION_ID),
                OsString::from("--json-argv"),
                OsString::from(&payload),
            ],
            &mut Cursor::new(Vec::new()),
        )
        .unwrap();
        assert_eq!(marked, payload.as_bytes());
        assert_eq!(
            read_hook_payload(
                &[
                    OsString::from("--integration-id"),
                    OsString::from("other"),
                    OsString::from("--json-stdin"),
                ],
                &mut Cursor::new(payload.as_bytes()),
            )
            .unwrap_err()
            .exit_code,
            HookExitCode::Usage
        );
    }
}
