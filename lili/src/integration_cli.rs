use std::ffi::OsString;

use lili_integration::{build_install_plan, inspect, install, load_plan};
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
    if arguments.len() != 2 || !matches!(subcommand, Some("inspect" | "plan")) {
        eprintln!("usage: lili integrate <inspect|plan|install --plan <path>>");
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
        serde_json::to_value(build_install_plan(
            &inspection,
            &hook_binary,
            unix_time_ms(),
        ))
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

fn run_install(arguments: &[OsString]) -> u8 {
    let [_, _, flag, path] = arguments else {
        eprintln!("usage: lili integrate install --plan <path>");
        return 2;
    };
    if flag != "--plan" {
        eprintln!("usage: lili integrate install --plan <path>");
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
