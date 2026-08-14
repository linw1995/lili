use std::ffi::OsString;

use lili_integration::{
    PluginMigrationAssessment, PluginMigrationEvidence, assess_plugin_migration,
    build_coexistence_install_plan, build_install_plan, cleanup_legacy_after_verification, inspect,
    inspect_plugin, install, install_plugin, load_plan, load_plugin_migration_assessment,
    plugin_hooks_are_trusted, save_plugin_migration_verification, uninstall,
};
use lili_pet::resolve_codex_home;
use lili_session::{
    DESKTOP_VERSION, ForwardingAckDisposition, ForwardingCredentialStore,
    ProviderCapabilitiesInputV1, ProviderInputV1, deliver_forwarding_message,
    normalize_provider_input,
};

const SYNTHETIC_DELIVERY_DEADLINE: std::time::Duration = std::time::Duration::from_millis(750);

pub fn try_run(arguments: &[OsString]) -> Option<u8> {
    let command = arguments.first()?;
    if command != "integrate" {
        return None;
    }
    let subcommand = arguments.get(1).and_then(|argument| argument.to_str());
    if subcommand == Some("install") {
        return Some(run_install(arguments));
    }
    if subcommand == Some("uninstall") {
        return Some(run_uninstall(arguments));
    }
    if subcommand == Some("cleanup") {
        return Some(run_cleanup(arguments));
    }
    if subcommand == Some("assess") {
        return Some(run_assess(arguments));
    }
    let legacy_plan = arguments.len() == 3
        && subcommand == Some("plan")
        && arguments
            .get(2)
            .is_some_and(|argument| argument == "--legacy-fallback");
    let coexist = arguments.len() == 4 && legacy_plan_arguments(arguments, "--coexist");
    if !(arguments.len() == 2 && subcommand == Some("inspect")) && !legacy_plan && !coexist {
        eprintln!(
            "usage: lili integrate <inspect|assess --plugin <plugin@marketplace>|plan --legacy-fallback [--coexist]|install <--assessment <path>|--legacy-fallback --plan <path>>|cleanup --assessment <path>|uninstall>"
        );
        return Some(2);
    }
    let codex_home = match resolve_codex_home() {
        Ok(codex_home) => codex_home,
        Err(error) => {
            eprintln!("Codex home could not be resolved: {error}");
            return Some(3);
        }
    };
    let inspection = inspect(&codex_home);
    let output = if subcommand == Some("inspect") {
        serde_json::to_value(inspection)
    } else {
        let hook_binary = packaged_hook_binary();
        let timestamp_ms = unix_time_ms();
        if coexist {
            serde_json::to_value(build_coexistence_install_plan(
                &inspection,
                &hook_binary,
                timestamp_ms,
            ))
        } else {
            serde_json::to_value(build_install_plan(&inspection, &hook_binary, timestamp_ms))
        }
    };
    let output = match output {
        Ok(output) => output,
        Err(_) => {
            eprintln!("integration output could not be serialized");
            return Some(4);
        }
    };
    match serde_json::to_writer_pretty(std::io::stdout().lock(), &output) {
        Ok(()) => {
            println!();
            Some(0)
        }
        Err(_) => {
            eprintln!("integration inspection could not be written");
            Some(4)
        }
    }
}

fn run_assess(arguments: &[OsString]) -> u8 {
    let selector = match assess_selector(arguments) {
        Ok(selector) => selector,
        Err(()) => return 2,
    };
    let codex_home = match resolve_codex_home() {
        Ok(codex_home) => codex_home,
        Err(error) => {
            eprintln!("Codex home could not be resolved: {error}");
            return 3;
        }
    };
    let assessment = match build_runtime_assessment(&codex_home, selector) {
        Ok(assessment) => assessment,
        Err(error) => {
            eprintln!("plugin migration verification failed: {error}");
            return 5;
        }
    };
    write_assessment(&assessment)
}

fn assess_selector(arguments: &[OsString]) -> Result<&str, ()> {
    let [_, _, flag, selector] = arguments else {
        eprintln!("usage: lili integrate assess --plugin <plugin@marketplace>");
        return Err(());
    };
    if flag != "--plugin" {
        eprintln!("usage: lili integrate assess --plugin <plugin@marketplace>");
        return Err(());
    }
    selector.to_str().ok_or_else(|| {
        eprintln!("plugin selector is invalid");
    })
}

fn build_runtime_assessment(
    codex_home: &std::path::Path,
    selector: &str,
) -> Result<PluginMigrationAssessment, lili_integration::PluginMigrationError> {
    let inspection = inspect_plugin(codex_home, selector);
    let (evidence, synthetic_event_id, credentials) =
        collect_runtime_verification(codex_home, selector, &inspection);
    let assessment =
        assess_plugin_migration(&inspection, &inspection.codex_adapter, selector, &evidence);
    save_cleanup_verification_if_ready(
        codex_home,
        &assessment,
        credentials.as_ref(),
        &synthetic_event_id,
    )?;
    Ok(assessment)
}

fn collect_runtime_verification(
    codex_home: &std::path::Path,
    selector: &str,
    inspection: &lili_integration::IntegrationInspection,
) -> (
    PluginMigrationEvidence,
    String,
    Option<lili_session::ForwardingCredentials>,
) {
    let hooks_reviewed = plugin_hooks_are_trusted(codex_home, selector);
    let (synthetic, overlap, event_id, credentials) = if hooks_reviewed
        && inspection
            .codex_adapter
            .plugin
            .last_accepted_plugin_event
            .is_some()
    {
        verify_synthetic_overlap(codex_home, selector)
    } else {
        (false, false, String::new(), None)
    };
    (
        PluginMigrationEvidence {
            exact_hooks_reviewed_by_user: hooks_reviewed,
            synthetic_delivery_verified: synthetic,
            overlap_deduplication_verified: overlap,
        },
        event_id,
        credentials,
    )
}

fn save_cleanup_verification_if_ready(
    codex_home: &std::path::Path,
    assessment: &PluginMigrationAssessment,
    credentials: Option<&lili_session::ForwardingCredentials>,
    synthetic_event_id: &str,
) -> Result<(), lili_integration::PluginMigrationError> {
    if !assessment.cleanup_allowed() {
        return Ok(());
    }
    let credentials =
        credentials.ok_or(lili_integration::PluginMigrationError::FailedPrecondition)?;
    save_plugin_migration_verification(codex_home, assessment, credentials, synthetic_event_id)?;
    Ok(())
}

fn write_assessment(assessment: &PluginMigrationAssessment) -> u8 {
    match serde_json::to_writer_pretty(std::io::stdout().lock(), &assessment) {
        Ok(()) => {
            println!();
            0
        }
        Err(_) => 4,
    }
}

fn verify_synthetic_overlap(
    codex_home: &std::path::Path,
    plugin_selector: &str,
) -> (
    bool,
    bool,
    String,
    Option<lili_session::ForwardingCredentials>,
) {
    let record =
        match ForwardingCredentialStore::for_runtime_dir(&codex_home.join("lili").join("runtime"))
            .load()
        {
            Ok(record) => record,
            Err(_) => return (false, false, String::new(), None),
        };
    let credentials = match record.credentials() {
        Ok(credentials) => credentials,
        Err(_) => return (false, false, String::new(), None),
    };
    let now_ms = unix_time_ms();
    let event_id = format!("lili-migration-verification-{now_ms}");
    let input = |source_discriminator: &str| ProviderInputV1 {
        version: 1,
        provider: Some("codex".to_owned()),
        event_type: Some("turn_completed".to_owned()),
        event_id: Some(event_id.clone()),
        session_id: Some(format!("lili-migration-{now_ms}")),
        turn_id: Some("verification".to_owned()),
        occurred_at_ms: Some(now_ms),
        project: None,
        summary: None,
        capabilities: ProviderCapabilitiesInputV1::default(),
        source_discriminator: Some(source_discriminator.to_owned()),
    };
    let Ok(legacy_event) = normalize_provider_input(input("hook:Stop")) else {
        return (false, false, String::new(), None);
    };
    let plugin_source = format!("plugin:{plugin_selector}:{DESKTOP_VERSION}:hook:Stop");
    let Ok(plugin_event) = normalize_provider_input(input(&plugin_source)) else {
        return (false, false, String::new(), None);
    };
    let Ok(legacy_message) = credentials.sign_verification(legacy_event, now_ms) else {
        return (false, false, String::new(), None);
    };
    let Ok(plugin_message) = credentials.sign_verification(plugin_event, now_ms) else {
        return (false, false, String::new(), None);
    };
    let delivered = tauri::async_runtime::block_on(async {
        let first = tokio::time::timeout(
            SYNTHETIC_DELIVERY_DEADLINE,
            deliver_forwarding_message(&record, &legacy_message),
        )
        .await;
        let second = tokio::time::timeout(
            SYNTHETIC_DELIVERY_DEADLINE,
            deliver_forwarding_message(&record, &plugin_message),
        )
        .await;
        (first, second)
    });
    let synthetic_delivery_verified = delivered.0.is_ok_and(|result| {
        result.is_ok_and(|ack| ack.disposition() == ForwardingAckDisposition::Accepted)
    });
    let overlap_deduplication_verified = delivered.1.is_ok_and(|result| {
        result.is_ok_and(|ack| ack.disposition() == ForwardingAckDisposition::Duplicate)
    });
    (
        synthetic_delivery_verified,
        synthetic_delivery_verified && overlap_deduplication_verified,
        event_id,
        Some(credentials),
    )
}

fn run_cleanup(arguments: &[OsString]) -> u8 {
    let [_, _, flag, path] = arguments else {
        eprintln!("usage: lili integrate cleanup --assessment <path>");
        return 2;
    };
    if flag != "--assessment" {
        eprintln!("usage: lili integrate cleanup --assessment <path>");
        return 2;
    }
    let assessment = match load_plugin_migration_assessment(std::path::Path::new(path)) {
        Ok(assessment) => assessment,
        Err(error) => {
            eprintln!("plugin migration assessment could not be loaded: {error}");
            return 3;
        }
    };
    let codex_home = match resolve_codex_home() {
        Ok(codex_home) => codex_home,
        Err(error) => {
            eprintln!("Codex home could not be resolved: {error}");
            return 3;
        }
    };
    match cleanup_legacy_after_verification(&codex_home, &assessment) {
        Ok(outcome) => match serde_json::to_writer_pretty(std::io::stdout().lock(), &outcome) {
            Ok(()) => {
                println!();
                0
            }
            Err(_) => 4,
        },
        Err(error) => {
            eprintln!("plugin migration cleanup failed: {error}");
            5
        }
    }
}

fn legacy_plan_arguments(arguments: &[OsString], final_flag: &str) -> bool {
    arguments
        .get(2)
        .is_some_and(|argument| argument == "--legacy-fallback")
        && arguments
            .get(3)
            .is_some_and(|argument| argument == final_flag)
}

fn run_uninstall(arguments: &[OsString]) -> u8 {
    if arguments.len() != 2 {
        eprintln!("usage: lili integrate uninstall");
        return 2;
    }
    let codex_home = match resolve_codex_home() {
        Ok(codex_home) => codex_home,
        Err(error) => {
            eprintln!("Codex home could not be resolved: {error}");
            return 3;
        }
    };
    match uninstall(&codex_home) {
        Ok(outcome) => match serde_json::to_writer_pretty(std::io::stdout().lock(), &outcome) {
            Ok(()) => {
                println!();
                0
            }
            Err(_) => 4,
        },
        Err(error) => {
            eprintln!("integration uninstall failed: {error}");
            5
        }
    }
}

fn run_install(arguments: &[OsString]) -> u8 {
    if let [_, _, flag, path] = arguments
        && flag == "--assessment"
    {
        return run_plugin_install(std::path::Path::new(path));
    }
    let [_, _, legacy, flag, path] = arguments else {
        eprintln!(
            "usage: lili integrate install <--assessment <path>|--legacy-fallback --plan <path>>"
        );
        return 2;
    };
    if legacy != "--legacy-fallback" || flag != "--plan" {
        eprintln!(
            "usage: lili integrate install <--assessment <path>|--legacy-fallback --plan <path>>"
        );
        return 2;
    }
    let plan = match load_plan(std::path::Path::new(path)) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("integration plan could not be loaded: {error}");
            return 3;
        }
    };
    match install(&plan) {
        Ok(outcome) => match serde_json::to_writer_pretty(std::io::stdout().lock(), &outcome) {
            Ok(()) => {
                println!();
                0
            }
            Err(_) => 4,
        },
        Err(error) => {
            eprintln!("integration install failed: {error}");
            5
        }
    }
}

fn run_plugin_install(path: &std::path::Path) -> u8 {
    match execute_plugin_install(path) {
        Ok(inspection) => write_json(&inspection),
        Err((exit_code, diagnostic)) => {
            eprintln!("{diagnostic}");
            exit_code
        }
    }
}

fn execute_plugin_install(
    path: &std::path::Path,
) -> Result<lili_integration::IntegrationInspection, (u8, String)> {
    let assessment = load_plugin_migration_assessment(path).map_err(|error| {
        (
            3,
            format!("plugin migration assessment could not be loaded: {error}"),
        )
    })?;
    let codex_home = resolve_codex_home()
        .map_err(|error| (3, format!("Codex home could not be resolved: {error}")))?;
    install_plugin(&codex_home, &assessment)
        .map_err(|error| (5, format!("plugin migration install failed: {error}")))
}

fn write_json(value: &impl serde::Serialize) -> u8 {
    match serde_json::to_writer_pretty(std::io::stdout().lock(), value) {
        Ok(()) => {
            println!();
            0
        }
        Err(_) => 4,
    }
}

fn packaged_hook_binary() -> std::path::PathBuf {
    let current = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("lili"));
    let suffix = std::env::consts::EXE_SUFFIX;
    current.with_file_name(format!("lili-hook{suffix}"))
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
        sync::atomic::{AtomicU64, Ordering},
    };

    use lili_session::{
        BoundForwardingEndpoint, ForwardingAckDisposition, ForwardingPurpose, ForwardingVerifier,
    };

    use super::*;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "lili-integration-cli-{}-{sequence}",
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

    #[test]
    fn install_command_rejects_invalid_arguments_and_missing_plans() {
        assert_eq!(run_install(&[]), 2);
        assert_eq!(
            run_install(&[
                "integrate".into(),
                "install".into(),
                "--legacy-fallback".into(),
                "--invalid".into(),
                "plan.json".into(),
            ]),
            2
        );
        assert_eq!(
            run_install(&[
                "integrate".into(),
                "install".into(),
                "--assessment".into(),
                std::env::temp_dir()
                    .join("lili-missing-plugin-assessment.json")
                    .into_os_string(),
            ]),
            3
        );
        assert_eq!(
            run_install(&[
                "integrate".into(),
                "install".into(),
                "--legacy-fallback".into(),
                "--plan".into(),
                std::env::temp_dir()
                    .join("lili-missing-install-plan.json")
                    .into_os_string(),
            ]),
            3
        );
        assert_eq!(
            run_install(&[
                "integrate".into(),
                "install".into(),
                "--plan".into(),
                "plan.json".into(),
            ]),
            2
        );
    }

    #[test]
    fn cleanup_command_requires_a_bounded_assessment_file() {
        assert_eq!(run_cleanup(&[]), 2);
        assert_eq!(
            run_cleanup(&[
                "integrate".into(),
                "cleanup".into(),
                "--invalid".into(),
                "assessment.json".into(),
            ]),
            2
        );
        assert_eq!(
            run_cleanup(&[
                "integrate".into(),
                "cleanup".into(),
                "--assessment".into(),
                std::env::temp_dir()
                    .join("lili-missing-plugin-assessment.json")
                    .into_os_string(),
            ]),
            3
        );
    }

    #[test]
    fn assessment_command_requires_an_exact_plugin_selector_argument() {
        assert_eq!(run_assess(&[]), 2);
        assert_eq!(
            run_assess(&[
                "integrate".into(),
                "assess".into(),
                "--invalid".into(),
                "lili@test-marketplace".into(),
            ]),
            2
        );
    }

    #[test]
    fn synthetic_overlap_uses_the_authenticated_runtime_and_observes_a_duplicate() {
        let temp = TempDir::new();
        let runtime_dir = temp.0.join("lili/runtime");
        let endpoint = tauri::async_runtime::block_on(async {
            BoundForwardingEndpoint::bind(&runtime_dir).unwrap()
        });
        let credentials = endpoint.credentials();
        let expected_instance_id = credentials.instance_id().to_owned();
        let server = tauri::async_runtime::spawn(async move {
            let mut verifier = ForwardingVerifier::new(credentials);
            let mut first_event_id = None;
            for disposition in [
                ForwardingAckDisposition::Accepted,
                ForwardingAckDisposition::Duplicate,
            ] {
                let mut connection = endpoint.accept().await.unwrap();
                let payload = connection.read_payload().await.unwrap();
                let verified = verifier.verify_payload(&payload, unix_time_ms()).unwrap();
                assert_eq!(verified.purpose(), ForwardingPurpose::Verification);
                if let Some(first_event_id) = &first_event_id {
                    assert_eq!(verified.event().event_id, *first_event_id);
                } else {
                    first_event_id = Some(verified.event().event_id.clone());
                }
                connection
                    .write_acknowledgement(&verified.acknowledgement(disposition))
                    .await
                    .unwrap();
            }
        });

        let (synthetic, overlap, event_id, returned_credentials) =
            verify_synthetic_overlap(&temp.0, "lili@test-marketplace");
        tauri::async_runtime::block_on(server).unwrap();
        assert!(synthetic);
        assert!(overlap);
        assert!(event_id.starts_with("lili-migration-verification-"));
        assert_eq!(
            returned_credentials.unwrap().instance_id(),
            expected_instance_id
        );
    }

    #[test]
    fn synthetic_overlap_times_out_against_an_unresponsive_runtime() {
        let temp = TempDir::new();
        let runtime_dir = temp.0.join("lili/runtime");
        let _endpoint = tauri::async_runtime::block_on(async {
            BoundForwardingEndpoint::bind(&runtime_dir).unwrap()
        });
        let started = std::time::Instant::now();

        let (synthetic, overlap, _, _) = verify_synthetic_overlap(&temp.0, "lili@test-marketplace");

        assert!(!synthetic);
        assert!(!overlap);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
}
