use std::ffi::OsString;

use lili_integration::inspect;
use lili_pet::resolve_codex_home;

pub fn try_run(arguments: &[OsString]) -> Option<u8> {
    let [command, subcommand] = arguments else {
        return None;
    };
    if command != "integrate" {
        return None;
    }
    if subcommand != "inspect" {
        eprintln!("usage: lili integrate inspect");
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
    match serde_json::to_writer_pretty(std::io::stdout().lock(), &inspection) {
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
