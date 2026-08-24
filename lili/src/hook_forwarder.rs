use std::{
    ffi::OsString,
    io::Read,
    process::{Command, Stdio},
    time::Duration,
};

use lili_integration::LILI_INTEGRATION_ID;
use lili_session::{
    ForwardingCredentialStore, MAX_PROVIDER_PAYLOAD_BYTES, SpoolEnqueueOutcome, SpoolError,
    SqliteSpoolStore, deliver_forwarding_message, mark_plugin_hook_event, normalize_hook_json,
};
use lili_storage::ApplicationPaths;

pub const CONNECTION_DEADLINE: Duration = Duration::from_millis(50);
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

    fn isolated_success() -> Self {
        Self {
            exit_code: HookExitCode::Success,
            outcome: None,
            diagnostic: None,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum HookInvocation {
    Direct {
        payload: Vec<u8>,
        plugin_id: Option<String>,
    },
    Coexist {
        original_argv: Vec<String>,
        payload: Vec<u8>,
    },
}

pub async fn run_from_environment() -> HookResult {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let mut stdin = std::io::stdin().lock();
    let invocation = match read_hook_invocation(&arguments, &mut stdin) {
        Ok(invocation) => invocation,
        Err(result) => return result,
    };
    match invocation {
        HookInvocation::Direct { payload, plugin_id } => {
            if plugin_id.is_some() && plugin_id.as_deref().is_some_and(str::is_empty) {
                return HookResult::failure(
                    HookExitCode::InvalidInput,
                    "plugin invocation identity is invalid",
                );
            }
            let application_paths = match ApplicationPaths::resolve() {
                Ok(application_paths) => application_paths,
                Err(_) => return application_storage_failure(),
            };
            process_payload_with_source(
                &application_paths,
                &payload,
                unix_time_ms(),
                plugin_id.as_deref(),
            )
            .await
        }
        HookInvocation::Coexist {
            original_argv,
            payload,
        } => {
            let original_started = launch_original_notify(&original_argv, &payload);
            let lili_result = match ApplicationPaths::resolve() {
                Ok(application_paths) => {
                    process_payload(&application_paths, &payload, unix_time_ms()).await
                }
                Err(_) => application_storage_failure(),
            };
            isolate_coexistence_result(original_started, lili_result)
        }
    }
}

fn application_storage_failure() -> HookResult {
    HookResult::failure(
        HookExitCode::DeliveryFailed,
        "Lili application storage could not be resolved",
    )
}

pub async fn process_payload(
    application_paths: &ApplicationPaths,
    payload: &[u8],
    now_ms: u64,
) -> HookResult {
    process_payload_with_source(application_paths, payload, now_ms, None).await
}

async fn process_payload_with_source(
    application_paths: &ApplicationPaths,
    payload: &[u8],
    now_ms: u64,
    plugin_id: Option<&str>,
) -> HookResult {
    let mut event = match normalize_hook_json(payload, now_ms) {
        Ok(event) => event,
        Err(_) => {
            return HookResult::failure(
                HookExitCode::InvalidInput,
                "hook payload is invalid or unsupported",
            );
        }
    };
    if plugin_id.is_some_and(|plugin_id| !mark_plugin_hook_event(&mut event, plugin_id)) {
        return HookResult::failure(
            HookExitCode::InvalidInput,
            "plugin invocation requires a supported Codex lifecycle event",
        );
    }

    let credential_store =
        ForwardingCredentialStore::for_runtime_dir(&application_paths.runtime_root());
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

    match SqliteSpoolStore::for_hook(application_paths.clone()).enqueue(&event, now_ms) {
        Ok(SpoolEnqueueOutcome::Stored) => HookResult::success(HookOutcome::Spooled),
        Ok(SpoolEnqueueOutcome::DroppedByLimit) => HookResult::failure(
            HookExitCode::DeliveryFailed,
            "hook event was dropped by the offline spool limit",
        ),
        Err(error) => {
            let diagnostic = match error {
                SpoolError::Database(message) if message.contains("locked") => {
                    "hook SQLite spool was locked"
                }
                SpoolError::Database(message) if message.contains("migration") => {
                    "hook SQLite migration was unavailable"
                }
                SpoolError::Database(_) => "hook SQLite spool was unavailable",
                SpoolError::Io(_) => "hook local spool I/O failed",
                _ => "hook event could not be delivered or spooled",
            };
            HookResult::failure(HookExitCode::DeliveryFailed, diagnostic)
        }
    }
}

#[cfg(test)]
fn read_hook_payload<R: Read>(
    arguments: &[OsString],
    stdin: &mut R,
) -> Result<Vec<u8>, HookResult> {
    match read_hook_invocation(arguments, stdin)? {
        HookInvocation::Direct { payload, .. } => Ok(payload),
        HookInvocation::Coexist { .. } => Err(HookResult::failure(
            HookExitCode::Usage,
            "coexistence invocation is not a direct hook payload",
        )),
    }
}

fn read_hook_invocation<R: Read>(
    arguments: &[OsString],
    stdin: &mut R,
) -> Result<HookInvocation, HookResult> {
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
        [mode, encoded_argv, payload] if mode == "--coexist-notify-json" => {
            let encoded_argv = encoded_argv.to_str().ok_or_else(|| {
                HookResult::failure(
                    HookExitCode::InvalidInput,
                    "coexistence argv must be valid UTF-8",
                )
            })?;
            let original_argv =
                serde_json::from_str::<Vec<String>>(encoded_argv).map_err(|_| {
                    HookResult::failure(
                        HookExitCode::InvalidInput,
                        "coexistence argv must be a JSON string array",
                    )
                })?;
            if original_argv.first().is_none_or(String::is_empty) {
                return Err(HookResult::failure(
                    HookExitCode::InvalidInput,
                    "coexistence argv must contain a command",
                ));
            }
            let payload = bounded_argv_payload(payload)?;
            Ok(HookInvocation::Coexist {
                original_argv,
                payload,
            })
        }
        [mode, payload] if mode == "--json-argv" => {
            bounded_argv_payload(payload).map(|payload| HookInvocation::Direct {
                payload,
                plugin_id: None,
            })
        }
        [mode] if mode == "--json-stdin" => {
            bounded_stdin_payload(stdin).map(|payload| HookInvocation::Direct {
                payload,
                plugin_id: None,
            })
        }
        [plugin, plugin_id, mode] if plugin == "--plugin-hook" && mode == "--json-stdin" => {
            let plugin_id = plugin_id.to_str().ok_or_else(|| {
                HookResult::failure(
                    HookExitCode::InvalidInput,
                    "plugin invocation identity must be valid UTF-8",
                )
            })?;
            if plugin_id.is_empty() || plugin_id.len() > 128 {
                return Err(HookResult::failure(
                    HookExitCode::InvalidInput,
                    "plugin invocation identity is invalid",
                ));
            }
            bounded_stdin_payload(stdin).map(|payload| HookInvocation::Direct {
                payload,
                plugin_id: Some(plugin_id.to_owned()),
            })
        }
        _ => Err(HookResult::failure(
            HookExitCode::Usage,
            "usage: lili-hook [--plugin-hook <plugin-id>] --json-stdin | --json-argv <json>",
        )),
    }
}

fn bounded_stdin_payload<R: Read>(stdin: &mut R) -> Result<Vec<u8>, HookResult> {
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

fn bounded_argv_payload(payload: &OsString) -> Result<Vec<u8>, HookResult> {
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

fn launch_original_notify(argv: &[String], payload: &[u8]) -> bool {
    let Some(program) = argv.first().filter(|program| !program.is_empty()) else {
        return false;
    };
    let Ok(payload) = std::str::from_utf8(payload) else {
        return false;
    };
    Command::new(program)
        .args(&argv[1..])
        .arg(payload)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

fn isolate_coexistence_result(original_started: bool, lili_result: HookResult) -> HookResult {
    if lili_result.exit_code == HookExitCode::Success {
        lili_result
    } else if original_started {
        HookResult::isolated_success()
    } else {
        lili_result
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

    #[test]
    fn coexistence_preserves_original_argv_and_appends_one_payload() {
        let payload = String::from_utf8(payload()).unwrap();
        let encoded = serde_json::to_string(&vec![
            "existing-notifier".to_owned(),
            "--channel".to_owned(),
            "pet".to_owned(),
        ])
        .unwrap();
        let invocation = read_hook_invocation(
            &[
                OsString::from("--integration-id"),
                OsString::from(LILI_INTEGRATION_ID),
                OsString::from("--coexist-notify-json"),
                OsString::from(encoded),
                OsString::from(&payload),
            ],
            &mut Cursor::new(Vec::new()),
        )
        .unwrap();
        assert_eq!(
            invocation,
            HookInvocation::Coexist {
                original_argv: vec![
                    "existing-notifier".to_owned(),
                    "--channel".to_owned(),
                    "pet".to_owned(),
                ],
                payload: payload.into_bytes(),
            }
        );
    }

    #[test]
    fn plugin_marker_is_explicit_and_stdin_only() {
        let invocation = read_hook_invocation(
            &[
                OsString::from("--integration-id"),
                OsString::from(LILI_INTEGRATION_ID),
                OsString::from("--plugin-hook"),
                OsString::from("lili@lili-local"),
                OsString::from("--json-stdin"),
            ],
            &mut Cursor::new(
                br#"{"hook_event_name":"Stop","session_id":"session-1","turn_id":"turn-1"}"#,
            ),
        )
        .unwrap();
        assert!(matches!(
            invocation,
            HookInvocation::Direct {
                plugin_id: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn coexistence_failures_do_not_mask_the_other_delivery() {
        let lili_failure = HookResult::failure(HookExitCode::DeliveryFailed, "failed");
        assert_eq!(
            isolate_coexistence_result(true, lili_failure).exit_code,
            HookExitCode::Success
        );
        let lili_success = HookResult::success(HookOutcome::Delivered);
        assert_eq!(
            isolate_coexistence_result(false, lili_success).exit_code,
            HookExitCode::Success
        );
    }

    #[tokio::test]
    async fn unavailable_endpoint_falls_back_to_one_normalized_spool_record() {
        let temp = TempDir::new();
        let paths = ApplicationPaths::from_root(temp.0.clone()).unwrap();
        let result = process_payload(&paths, &payload(), 1_000).await;
        assert_eq!(result, HookResult::success(HookOutcome::Spooled));
        let spool = SqliteSpoolStore::for_application(paths);
        let claim = spool.claim_next(1_001).unwrap().unwrap();
        assert_eq!(claim.event().event_id.as_str(), "event-1");
        claim.commit().unwrap();
    }

    #[tokio::test]
    async fn plugin_invocation_spools_attributed_event_without_changing_identity() {
        let temp = TempDir::new();
        let paths = ApplicationPaths::from_root(temp.0.clone()).unwrap();
        let lifecycle =
            br#"{"hook_event_name":"Stop","session_id":"session-1","turn_id":"turn-1"}"#;
        let legacy = normalize_hook_json(lifecycle, 1_000).unwrap();
        let result =
            process_payload_with_source(&paths, lifecycle, 1_000, Some("lili@lili-local")).await;
        assert_eq!(result, HookResult::success(HookOutcome::Spooled));
        let spool = SqliteSpoolStore::for_application(paths);
        let claim = spool.claim_next(1_001).unwrap().unwrap();
        assert_eq!(claim.event().event_id, legacy.event_id);
        assert!(claim.event().source_discriminator.starts_with("plugin:"));
        claim.commit().unwrap();
    }

    #[tokio::test]
    async fn malformed_payload_has_a_deterministic_exit_code_without_spooling() {
        let temp = TempDir::new();
        let paths = ApplicationPaths::from_root(temp.0.clone()).unwrap();
        let result = process_payload(&paths, b"not-json", 1_000).await;
        assert_eq!(result.exit_code, HookExitCode::InvalidInput);
        assert!(!paths.database_path().exists());
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
