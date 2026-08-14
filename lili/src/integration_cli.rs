use std::ffi::OsString;

use lili_integration::{
    build_coexistence_install_plan, build_install_plan, cleanup_legacy_after_verification, inspect,
    install, load_plan, load_plugin_migration_assessment, uninstall,
};
use lili_pet::resolve_codex_home;

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
    let legacy_plan = arguments.len() == 3
        && subcommand == Some("plan")
        && arguments
            .get(2)
            .is_some_and(|argument| argument == "--legacy-fallback");
    let coexist = arguments.len() == 4 && legacy_plan_arguments(arguments, "--coexist");
    if !(arguments.len() == 2 && subcommand == Some("inspect")) && !legacy_plan && !coexist {
        eprintln!(
            "usage: lili integrate <inspect|plan --legacy-fallback [--coexist]|install --legacy-fallback --plan <path>|cleanup --assessment <path>|uninstall>"
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
}
