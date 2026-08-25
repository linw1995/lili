use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use lili_integration::{
    InstallError, IntegrationKind, build_install_plan, inspect_with_version, install_with_verifier,
};
use lili_session::SqliteSpoolStore;
use lili_storage::ApplicationPaths;
use serde::Deserialize;

const MAX_MATRIX_BYTES: u64 = 64 * 1024;
const EXPECTED_SURFACES: [&str; 6] = [
    "agent-turn-complete",
    "SessionStart",
    "UserPromptSubmit",
    "PermissionRequest",
    "Stop",
    "SessionEnd",
];

fn main() {
    if let Err(error) = run() {
        eprintln!("Codex matrix acceptance failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = std::env::args_os().skip(1);
    let hook_binary = required_file(arguments.next(), "hook binary")?;
    let fixtures_root = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "missing fixture root".to_owned())?;
    if arguments.next().is_some() || !fixtures_root.is_dir() {
        return Err("matrix arguments are invalid".to_owned());
    }
    let matrix: VersionMatrix = read_json(&fixtures_root.join("matrix.json"))?;
    if matrix.schema_version != 1 || matrix.required.is_empty() {
        return Err("version matrix is empty or unsupported".to_owned());
    }
    for entry in &matrix.required {
        verify_version(&hook_binary, &fixtures_root, entry)?;
    }
    println!(
        "{{\"codexMatrixAcceptance\":\"passed\",\"versions\":{}}}",
        matrix.required.len()
    );
    Ok(())
}

fn verify_version(
    hook_binary: &Path,
    fixtures_root: &Path,
    entry: &RequiredVersion,
) -> Result<(), String> {
    if entry.fixtures.len() != EXPECTED_SURFACES.len()
        || EXPECTED_SURFACES
            .iter()
            .any(|surface| !entry.fixtures.contains_key(*surface))
    {
        return Err(format!(
            "{} does not cover every required surface",
            entry.codex_version
        ));
    }
    let version_root = fixtures_root.join(&entry.codex_version);
    let manifest: FixtureManifest = read_json(&version_root.join("manifest.json"))?;
    if manifest.codex_version != entry.codex_version
        || manifest.source_tag != entry.source_tag
        || !manifest
            .surfaces
            .iter()
            .map(String::as_str)
            .eq(EXPECTED_SURFACES)
    {
        return Err(format!(
            "{} fixture manifest does not match the release matrix",
            entry.codex_version
        ));
    }

    let workspace = AcceptanceWorkspace::new(&entry.codex_version)?;
    let inspection = inspect_with_version(workspace.path(), Some(entry.codex_version.clone()));
    if inspection.tested_codex_version != entry.codex_version {
        return Err(format!(
            "{} is not the adapter's declared tested version",
            entry.codex_version
        ));
    }
    let plan = build_install_plan(&inspection, hook_binary, unix_time_ms());
    install_with_verifier(&plan, |command| {
        let Some(program) = command.first() else {
            return Err(InstallError::InvalidPlan);
        };
        let mut process = Command::new(program);
        process.args(&command[1..]);
        configure_command(&mut process, &workspace);
        let output = process.output()?;
        if output.status.success() && output.stdout.is_empty() && output.stderr.is_empty() {
            Ok(())
        } else {
            Err(InstallError::VerificationFailed)
        }
    })
    .map_err(|error| format!("integration install failed: {error}"))?;
    let installed = inspect_with_version(workspace.path(), Some(entry.codex_version.clone()));
    if installed.notify.kind != IntegrationKind::Lili
        || installed
            .hook_surfaces
            .iter()
            .any(|surface| surface.lili_handlers != 1)
    {
        return Err("installed integration is incomplete".to_owned());
    }

    for (surface, file_name) in &entry.fixtures {
        let payload = fs::read(version_root.join(file_name))
            .map_err(|error| format!("{file_name} could not be read: {error}"))?;
        let output = if surface == "agent-turn-complete" {
            let mut command = Command::new(&plan.notify.argv[0]);
            command
                .args(&plan.notify.argv[1..])
                .arg(String::from_utf8(payload).map_err(|_| "notify fixture is not UTF-8")?);
            configure_command(&mut command, &workspace);
            command.output()
        } else {
            let mut command = Command::new(hook_binary);
            command
                .args([
                    "--integration-id",
                    lili_integration::LILI_INTEGRATION_ID,
                    "--json-stdin",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            configure_command(&mut command, &workspace);
            let mut child = command
                .spawn()
                .map_err(|error| format!("{surface} hook could not start: {error}"))?;
            child
                .stdin
                .take()
                .ok_or_else(|| format!("{surface} hook stdin is unavailable"))?
                .write_all(&payload)
                .map_err(|error| format!("{surface} fixture could not be written: {error}"))?;
            child.wait_with_output()
        }
        .map_err(|error| format!("{surface} hook failed to execute: {error}"))?;
        if !output.status.success() || !output.stdout.is_empty() || !output.stderr.is_empty() {
            return Err(format!("{surface} fixture no longer normalizes"));
        }
    }

    let spool = SqliteSpoolStore::for_application(workspace.application_paths());
    let mut accepted = 0;
    while let Some(claim) = spool
        .claim_next(unix_time_ms())
        .map_err(|error| format!("normalized spool could not be read: {error}"))?
    {
        if claim.event().provider.as_str() != "codex" {
            return Err("matrix produced a non-Codex event".to_owned());
        }
        accepted += 1;
        claim
            .commit()
            .map_err(|error| format!("normalized spool could not be committed: {error}"))?;
    }
    if accepted != EXPECTED_SURFACES.len() + 1 {
        return Err(format!(
            "{} normalized {accepted} events instead of the required fixtures plus install probe",
            entry.codex_version
        ));
    }
    Ok(())
}

fn configure_command(command: &mut Command, workspace: &AcceptanceWorkspace) {
    command
        .env("CODEX_HOME", workspace.path())
        .env("HOME", workspace.home())
        .env("XDG_STATE_HOME", workspace.home().join("state"))
        .env("LOCALAPPDATA", workspace.home().join("local-app-data"));
}

fn required_file(value: Option<std::ffi::OsString>, label: &str) -> Result<PathBuf, String> {
    let path = value
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing {label} path"))?;
    path.is_absolute()
        .then_some(path)
        .filter(|path| path.is_file())
        .ok_or_else(|| format!("{label} path is invalid"))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("fixture metadata failed: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_MATRIX_BYTES {
        return Err("fixture metadata is invalid".to_owned());
    }
    let contents = fs::read(path).map_err(|error| format!("fixture read failed: {error}"))?;
    serde_json::from_slice(&contents).map_err(|error| format!("fixture JSON is invalid: {error}"))
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VersionMatrix {
    schema_version: u16,
    required: Vec<RequiredVersion>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RequiredVersion {
    codex_version: String,
    source_tag: String,
    fixtures: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureManifest {
    codex_version: String,
    source_tag: String,
    surfaces: Vec<String>,
}

struct AcceptanceWorkspace {
    path: PathBuf,
    home: PathBuf,
}

impl AcceptanceWorkspace {
    fn new(version: &str) -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!(
            "lili-codex-matrix-{}-{}-{version}",
            std::process::id(),
            unix_time_ms()
        ));
        let home = path.join("home");
        fs::create_dir_all(&path)
            .map_err(|error| format!("matrix workspace could not be created: {error}"))?;
        fs::create_dir_all(&home)
            .map_err(|error| format!("matrix home could not be created: {error}"))?;
        Ok(Self { path, home })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn home(&self) -> &Path {
        &self.home
    }

    fn application_paths(&self) -> ApplicationPaths {
        #[cfg(target_os = "macos")]
        let root = self
            .home
            .join("Library")
            .join("Application Support")
            .join(lili_storage::APPLICATION_IDENTIFIER);
        #[cfg(target_os = "windows")]
        let root = self
            .home
            .join("local-app-data")
            .join(lili_storage::APPLICATION_IDENTIFIER);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let root = self
            .home
            .join("state")
            .join(lili_storage::APPLICATION_IDENTIFIER);
        ApplicationPaths::from_root(root).expect("matrix application path must be absolute")
    }
}

impl Drop for AcceptanceWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
