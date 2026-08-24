use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(not(target_os = "windows"))]
use std::io::Write;

use serde::Deserialize;

const EXPECTED_CODEX_VERSION: &str = "0.147.0";
const MARKETPLACE_NAME: &str = "lili-local";
const PLUGIN_SELECTOR: &str = "lili@lili-local";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
#[cfg(not(target_os = "windows"))]
const HOOK_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(target_os = "windows")]
const CODEX_HOOK_DISPATCH_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct AcceptanceTarget {
    pub triple: &'static str,
    pub forwarder_name: &'static str,
    pub os: &'static str,
    pub architecture: &'static str,
}

pub const MACOS_ARM64: AcceptanceTarget = AcceptanceTarget {
    triple: "arm64-apple-darwin",
    forwarder_name: "lili-hook",
    os: "macos",
    architecture: "aarch64",
};
pub const LINUX_X86_64: AcceptanceTarget = AcceptanceTarget {
    triple: "x86_64-unknown-linux-gnu",
    forwarder_name: "lili-hook",
    os: "linux",
    architecture: "x86_64",
};
pub const WINDOWS_X86_64: AcceptanceTarget = AcceptanceTarget {
    triple: "x86_64-pc-windows-msvc",
    forwarder_name: "lili-hook.exe",
    os: "windows",
    architecture: "x86_64",
};

#[derive(Debug)]
pub struct InstalledMarketplacePlugin {
    root: PathBuf,
}

impl InstalledMarketplacePlugin {
    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketplaceAddOutput {
    marketplace_name: String,
    installed_root: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginAddOutput {
    plugin_id: String,
    version: String,
    installed_path: PathBuf,
}

pub fn install_local_marketplace_plugin(
    codex_binary: &Path,
    repository_root: &Path,
    codex_home: &Path,
    forwarder: &Path,
    target: AcceptanceTarget,
) -> Result<InstalledMarketplacePlugin, String> {
    require_current_target(target)?;
    require_file(codex_binary, "Codex executable")?;
    require_file(forwarder, "plugin forwarder")?;
    let plugin_source = repository_root.join("plugins").join("lili");
    let catalog_source = repository_root
        .join("marketplace")
        .join("local")
        .join(".agents")
        .join("plugins")
        .join("marketplace.json");
    let package_policy = repository_root
        .join("marketplace")
        .join("lili")
        .join("package-policy.json");
    require_directory(&plugin_source, "plugin source")?;
    require_file(&catalog_source, "local Marketplace catalog")?;
    require_file(&package_policy, "plugin package policy")?;
    validate_declared_target(&package_policy, target)?;
    if plugin_source.join("bin").exists() {
        return Err("plugin source contains build outputs".to_owned());
    }

    let home = codex_home.join("acceptance-home");
    fs::create_dir_all(&home)
        .map_err(|error| format!("acceptance home could not be created: {error}"))?;
    let catalog_root = codex_home.join("acceptance-marketplace");
    if catalog_root.exists() {
        return Err("acceptance Marketplace already exists".to_owned());
    }
    let catalog_manifest = catalog_root
        .join(".agents")
        .join("plugins")
        .join("marketplace.json");
    fs::create_dir_all(
        catalog_manifest
            .parent()
            .expect("catalog manifest has a parent"),
    )
    .map_err(|error| format!("acceptance catalog could not be created: {error}"))?;
    fs::copy(&catalog_source, &catalog_manifest)
        .map_err(|error| format!("acceptance catalog could not be copied: {error}"))?;
    let staged_plugin = catalog_root.join("plugins").join("lili");
    fs::create_dir_all(staged_plugin.parent().expect("staged plugin has a parent"))
        .map_err(|error| format!("plugin catalog directory could not be created: {error}"))?;
    copy_tree(&plugin_source, &staged_plugin)?;
    let staged_forwarder = staged_plugin
        .join("bin")
        .join(target.triple)
        .join(target.forwarder_name);
    fs::create_dir_all(
        staged_forwarder
            .parent()
            .expect("staged forwarder has a parent"),
    )
    .map_err(|error| format!("forwarder target directory could not be created: {error}"))?;
    fs::copy(forwarder, &staged_forwarder)
        .map_err(|error| format!("plugin forwarder could not be staged: {error}"))?;

    let version = run_codex(codex_binary, codex_home, &home, ["--version"])?;
    if version.stdout != format!("codex-cli {EXPECTED_CODEX_VERSION}\n").as_bytes() {
        return Err("Codex version differs from the reviewed contract".to_owned());
    }

    let marketplace = run_codex(
        codex_binary,
        codex_home,
        &home,
        [
            OsString::from("plugin"),
            OsString::from("marketplace"),
            OsString::from("add"),
            catalog_root.as_os_str().to_owned(),
            OsString::from("--json"),
        ],
    )?;
    let marketplace: MarketplaceAddOutput = parse_json(&marketplace.stdout, "marketplace add")?;
    if marketplace.marketplace_name != MARKETPLACE_NAME
        || canonical(&marketplace.installed_root)? != canonical(&catalog_root)?
    {
        return Err("installed Marketplace identity drifted".to_owned());
    }

    let plugin = run_codex(
        codex_binary,
        codex_home,
        &home,
        ["plugin", "add", PLUGIN_SELECTOR, "--json"],
    )?;
    let plugin: PluginAddOutput = parse_json(&plugin.stdout, "plugin add")?;
    if plugin.plugin_id != PLUGIN_SELECTOR || plugin.version != env!("CARGO_PKG_VERSION") {
        return Err("installed plugin identity drifted".to_owned());
    }
    let installed_root = canonical(&plugin.installed_path)?;
    let cache_root = canonical(&codex_home.join("plugins").join("cache"))?;
    if !installed_root.starts_with(&cache_root) {
        return Err("installed plugin escaped the Codex plugin cache".to_owned());
    }
    require_file(
        &installed_root
            .join("bin")
            .join(target.triple)
            .join(target.forwarder_name),
        "installed plugin forwarder",
    )?;
    let installed_manifest = fs::read(installed_root.join(".codex-plugin").join("plugin.json"))
        .map_err(|error| format!("installed plugin manifest could not be read: {error}"))?;
    let manifest: serde_json::Value = parse_json(&installed_manifest, "installed plugin manifest")?;
    if manifest.get("name").and_then(serde_json::Value::as_str) != Some("lili")
        || manifest.get("version").and_then(serde_json::Value::as_str)
            != Some(env!("CARGO_PKG_VERSION"))
    {
        return Err("installed plugin manifest drifted".to_owned());
    }

    let packaged_forwarder = installed_root
        .join("bin")
        .join(target.triple)
        .join(target.forwarder_name);
    let forwarder_version = run_command(
        Command::new(&packaged_forwarder).arg("--version"),
        COMMAND_TIMEOUT,
        "installed plugin forwarder",
    )?;
    if !forwarder_version.stderr.is_empty()
        || forwarder_version.stdout
            != format!("lili-hook {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    {
        return Err("installed plugin forwarder version drifted".to_owned());
    }

    Ok(InstalledMarketplacePlugin {
        root: installed_root,
    })
}

pub fn invoke_installed_plugin_hook(
    plugin: &InstalledMarketplacePlugin,
    codex_home: &Path,
    application_home: &Path,
    payload: &[u8],
    codex_binary: &Path,
    repository_root: &Path,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let _ = payload;
        let script = repository_root.join("scripts").join("test_hook_trust.py");
        require_file(&script, "Codex hook dispatch script")?;
        let mut command = Command::new("python");
        command
            .arg(script)
            .arg("--workspace-root")
            .arg(repository_root)
            .arg("--codex")
            .arg(codex_binary)
            .arg("--installed-codex-home")
            .arg(codex_home)
            .arg("--installed-plugin-root")
            .arg(plugin.root())
            .arg("--dispatch-cwd")
            .arg(repository_root)
            .env("HOME", application_home)
            .env("LOCALAPPDATA", application_home)
            .env("XDG_STATE_HOME", application_home.join("state"));
        let output = run_command(
            &mut command,
            CODEX_HOOK_DISPATCH_TIMEOUT,
            "Codex Windows hook dispatch",
        )?;
        let result: serde_json::Value = parse_json(&output.stdout, "Codex Windows hook dispatch")?;
        if result.get("result").and_then(serde_json::Value::as_str) != Some("passed")
            || result
                .get("bypassUsed")
                .and_then(serde_json::Value::as_bool)
                != Some(false)
            || result.get("event").and_then(serde_json::Value::as_str) != Some("sessionStart")
        {
            return Err("Codex Windows hook dispatch contract failed".to_owned());
        }
        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (codex_binary, repository_root);
        let mut command = Command::new(plugin.root.join("hooks").join("forward"));

        command
            .env("PLUGIN_ROOT", &plugin.root)
            .env(
                "PLUGIN_DATA",
                codex_home
                    .join("plugins")
                    .join("data")
                    .join("lili-lili-local"),
            )
            .env("CODEX_HOME", codex_home)
            .env("HOME", application_home)
            .env("LOCALAPPDATA", application_home)
            .env("XDG_STATE_HOME", application_home.join("state"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| format!("installed plugin hook could not start: {error}"))?;
        child
            .stdin
            .take()
            .expect("piped hook stdin is available")
            .write_all(payload)
            .map_err(|error| format!("installed plugin hook input failed: {error}"))?;
        let output = wait_for_output(child, HOOK_TIMEOUT, "installed plugin hook")?;
        if !output.status.success() || !output.stdout.is_empty() || !output.stderr.is_empty() {
            return Err("installed plugin hook did not complete silently".to_owned());
        }
        Ok(())
    }
}

fn run_codex<I, S>(
    executable: &Path,
    codex_home: &Path,
    home: &Path,
    arguments: I,
) -> Result<Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .env("CODEX_HOME", codex_home)
        .env("HOME", home);
    run_command(&mut command, COMMAND_TIMEOUT, "Codex")
}

fn run_command(command: &mut Command, timeout: Duration, label: &str) -> Result<Output, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|error| format!("{label} could not start: {error}"))?;
    let output = wait_for_output(child, timeout, label)?;
    if output.stdout.len() > MAX_OUTPUT_BYTES || output.stderr.len() > MAX_OUTPUT_BYTES {
        return Err(format!("{label} output exceeded its bound"));
    }
    if !output.status.success() {
        return Err(format!(
            "{label} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output)
}

fn wait_for_output(mut child: Child, timeout: Duration, label: &str) -> Result<Output, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|error| format!("{label} output could not be collected: {error}"));
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{label} exceeded its execution budget"));
            }
            Err(error) => return Err(format!("{label} could not be observed: {error}")),
        }
    }
}

fn parse_json<T: serde::de::DeserializeOwned>(payload: &[u8], label: &str) -> Result<T, String> {
    if payload.len() > MAX_OUTPUT_BYTES {
        return Err(format!("{label} JSON exceeded its bound"));
    }
    serde_json::from_slice(payload).map_err(|error| format!("{label} JSON is invalid: {error}"))
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("plugin source metadata failed: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("plugin source tree is unsafe".to_owned());
    }
    fs::create_dir(destination)
        .map_err(|error| format!("plugin destination could not be created: {error}"))?;
    for entry in fs::read_dir(source)
        .map_err(|error| format!("plugin source could not be listed: {error}"))?
    {
        let entry = entry.map_err(|error| format!("plugin source entry failed: {error}"))?;
        let metadata = entry
            .file_type()
            .map_err(|error| format!("plugin source entry metadata failed: {error}"))?;
        let target = destination.join(entry.file_name());
        if metadata.is_symlink() {
            return Err("plugin source contains a symbolic link".to_owned());
        }
        if metadata.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if metadata.is_file() {
            fs::copy(entry.path(), target)
                .map_err(|error| format!("plugin source file could not be copied: {error}"))?;
        } else {
            return Err("plugin source contains an unsupported entry".to_owned());
        }
    }
    Ok(())
}

fn require_current_target(target: AcceptanceTarget) -> Result<(), String> {
    if std::env::consts::OS != target.os || std::env::consts::ARCH != target.architecture {
        return Err(format!(
            "acceptance target {} does not match the current host",
            target.triple
        ));
    }
    Ok(())
}

fn validate_declared_target(path: &Path, target: AcceptanceTarget) -> Result<(), String> {
    let payload = fs::read(path)
        .map_err(|error| format!("plugin package policy could not be read: {error}"))?;
    let policy: serde_json::Value = parse_json(&payload, "plugin package policy")?;
    let expected = format!("bin/{}/{}", target.triple, target.forwarder_name);
    let declared = policy
        .get("allowedPackageFiles")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|entries| {
            entries
                .iter()
                .any(|entry| entry.as_str() == Some(expected.as_str()))
        });
    if !declared {
        return Err(format!(
            "acceptance target {} is not declared by the package policy",
            target.triple
        ));
    }
    Ok(())
}

fn require_file(path: &Path, label: &str) -> Result<(), String> {
    path.is_file()
        .then_some(())
        .ok_or_else(|| format!("{label} path is not a file"))
}

fn require_directory(path: &Path, label: &str) -> Result<(), String> {
    path.is_dir()
        .then_some(())
        .ok_or_else(|| format!("{label} path is not a directory"))
}

fn canonical(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|error| format!("path could not be resolved: {error}"))
}
