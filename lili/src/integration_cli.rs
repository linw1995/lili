use std::ffi::OsString;

use lili_integration::{
    PluginMigrationEvidence, assess_plugin_migration, build_coexistence_install_plan,
    build_install_plan, cleanup_legacy_after_verification, inspect, inspect_plugin, install,
    load_plan, load_plugin_migration_assessment, plugin_hooks_are_trusted,
    save_plugin_migration_verification, uninstall,
};
use lili_pet::resolve_codex_home;
use lili_session::{
    ForwardingAckDisposition, ForwardingCredentialStore, ProviderCapabilitiesInputV1,
    ProviderInputV1, deliver_forwarding_message, normalize_provider_input,
};

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
            "usage: lili integrate <inspect|assess --plugin <plugin@marketplace>|plan --legacy-fallback [--coexist]|install --legacy-fallback --plan <path>|cleanup --assessment <path>|uninstall>"
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
    let [_, _, flag, selector] = arguments else {
        eprintln!("usage: lili integrate assess --plugin <plugin@marketplace>");
        return 2;
    };
    if flag != "--plugin" {
        eprintln!("usage: lili integrate assess --plugin <plugin@marketplace>");
        return 2;
    }
    let Some(selector) = selector.to_str() else {
        eprintln!("plugin selector is invalid");
        return 2;
    };
    let codex_home = match resolve_codex_home() {
        Ok(codex_home) => codex_home,
        Err(error) => {
            eprintln!("Codex home could not be resolved: {error}");
            return 3;
        }
    };
    let inspection = inspect_plugin(&codex_home, selector);
    let hooks_reviewed = plugin_hooks_are_trusted(&codex_home, selector);
    let (
        synthetic_delivery_verified,
        overlap_deduplication_verified,
        synthetic_event_id,
        credentials,
    ) = if hooks_reviewed
        && inspection
            .codex_adapter
            .plugin
            .last_accepted_plugin_event
            .is_some()
    {
        verify_synthetic_overlap(&codex_home)
    } else {
        (false, false, String::new(), None)
    };
    let assessment = assess_plugin_migration(
        &inspection,
        &inspection.codex_adapter,
        selector,
        &PluginMigrationEvidence {
            exact_hooks_reviewed_by_user: hooks_reviewed,
            synthetic_delivery_verified,
            overlap_deduplication_verified,
        },
    );
    if assessment.cleanup_allowed() {
        let Some(credentials) = credentials.as_ref() else {
            eprintln!("plugin migration verification could not be authenticated");
            return 5;
        };
        if let Err(error) = save_plugin_migration_verification(
            &codex_home,
            &assessment,
            credentials,
            &synthetic_event_id,
        ) {
            eprintln!("plugin migration verification could not be saved: {error}");
            return 5;
        }
    }
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
    let Ok(plugin_event) = normalize_provider_input(input("plugin:verification:hook:Stop")) else {
        return (false, false, String::new(), None);
    };
    let Ok(legacy_message) = credentials.sign(legacy_event, now_ms) else {
        return (false, false, String::new(), None);
    };
    let Ok(plugin_message) = credentials.sign(plugin_event, now_ms) else {
        return (false, false, String::new(), None);
    };
    let delivered = tauri::async_runtime::block_on(async {
        let first = deliver_forwarding_message(&record, &legacy_message).await;
        let second = deliver_forwarding_message(&record, &plugin_message).await;
        (first, second)
    });
    let synthetic_delivery_verified = delivered
        .0
        .is_ok_and(|ack| ack.disposition() == ForwardingAckDisposition::Accepted);
    let overlap_deduplication_verified = delivered
        .1
        .is_ok_and(|ack| ack.disposition() == ForwardingAckDisposition::Duplicate);
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
    let [_, _, legacy, flag, path] = arguments else {
        eprintln!("usage: lili integrate install --legacy-fallback --plan <path>");
        return 2;
    };
    if legacy != "--legacy-fallback" || flag != "--plan" {
        eprintln!("usage: lili integrate install --legacy-fallback --plan <path>");
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
    use super::*;

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
}
