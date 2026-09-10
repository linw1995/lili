#[cfg(target_os = "macos")]
fn main() {
    let mode = std::env::args_os().nth(1);
    let result = match mode.as_deref() {
        Some(mode) if mode == "--record-action" => macos::record_action(),
        Some(mode) if mode == "--title-action" => macos::title_action(),
        Some(mode) if mode == "--direct-hook" => macos::run_direct_hook_acceptance(),
        _ => macos::run(),
    };
    if let Err(error) = result {
        eprintln!("macOS acceptance failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("macOS acceptance requires macOS");
    std::process::exit(2);
}

#[cfg(target_os = "macos")]
mod macos {
    use std::{
        fs,
        io::{Read as _, Write as _},
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    use lili_lib::acceptance_marketplace::{
        MACOS_ARM64, install_local_marketplace_plugin, invoke_installed_plugin_hook,
    };
    use lili_storage::ApplicationPaths;

    const PAYLOAD: &[u8] = include_bytes!(
        "../../../lili-session/tests/fixtures/codex/0.147.0/permission-request.json"
    );

    pub fn title_action() -> Result<(), String> {
        let mut input = Vec::new();
        std::io::stdin()
            .take(16 * 1024 + 1)
            .read_to_end(&mut input)
            .map_err(|_| "title stdin failed".to_owned())?;
        if input.len() > 16 * 1024 {
            return Err("title stdin overflow".to_owned());
        }
        let request: serde_json::Value =
            serde_json::from_slice(&input).map_err(|_| "invalid title request".to_owned())?;
        if request["version"] != 1
            || request["trigger"] != "session_title"
            || !request["requestId"].is_string()
            || !request["sessionId"].is_string()
        {
            return Err("invalid title request fields".to_owned());
        }
        std::thread::sleep(Duration::from_secs(3));
        std::io::stdout()
            .write_all(br#"{"version":1,"title":"<b>Acceptance title</b>"}"#)
            .map_err(|_| "title stdout failed".to_owned())
    }

    pub fn record_action() -> Result<(), String> {
        let mut arguments = std::env::args_os().skip(1);
        if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--record-action")) {
            return Err("invalid action fixture mode".to_owned());
        }
        let output = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| "missing action fixture output".to_owned())?;
        if arguments.next().is_some() || !output.is_absolute() {
            return Err("invalid action fixture output".to_owned());
        }
        let mut input = Vec::new();
        std::io::stdin()
            .take((lili_actions::MAX_INTERACTION_CONTEXT_BYTES + 1) as u64)
            .read_to_end(&mut input)
            .map_err(|error| format!("action fixture stdin could not be read: {error}"))?;
        if input.len() > lili_actions::MAX_INTERACTION_CONTEXT_BYTES {
            return Err("action fixture stdin exceeded its bound".to_owned());
        }
        fs::write(output, input)
            .map_err(|error| format!("action fixture output could not be written: {error}"))
    }

    pub fn run() -> Result<(), String> {
        let mut arguments = std::env::args_os().skip(1);
        let app_binary = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| "missing packaged app binary path".to_owned())?;
        let hook_binary = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| "missing hook binary path".to_owned())?;
        let app_bundle = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| "missing packaged app bundle path".to_owned())?;
        let repository_root = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| "missing repository root path".to_owned())?;
        let codex_binary = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| "missing Codex binary path".to_owned())?;
        if arguments.next().is_some()
            || !app_binary.is_file()
            || !hook_binary.is_file()
            || !app_bundle.is_dir()
            || app_bundle.extension().and_then(|value| value.to_str()) != Some("app")
            || !repository_root.is_dir()
            || !codex_binary.is_file()
            || !app_binary
                .canonicalize()
                .map_err(|error| format!("packaged app binary could not be resolved: {error}"))?
                .starts_with(
                    app_bundle
                        .canonicalize()
                        .map_err(|error| format!("app bundle could not be resolved: {error}"))?,
                )
        {
            return Err("acceptance binary paths are invalid".to_owned());
        }

        let workspace = AcceptanceWorkspace::new()?;
        workspace.write_action_config()?;
        let plugin = install_local_marketplace_plugin(
            &codex_binary,
            &repository_root,
            workspace.path(),
            &hook_binary,
            MACOS_ARM64,
        )?;
        let mut app = spawn_app(&app_binary, workspace.path(), workspace.home(), false)?;
        let credential_path = workspace.application_paths().credentials_path();
        if !wait_for_file(&credential_path, Duration::from_secs(30)) {
            terminate(&mut app);
            return Err("packaged app did not publish forwarding credentials".to_owned());
        }

        if let Err(error) = invoke_installed_plugin_hook(
            &plugin,
            workspace.path(),
            workspace.home(),
            PAYLOAD,
            &codex_binary,
            &repository_root,
        ) {
            terminate(&mut app);
            return Err(error);
        }

        wait_for_completion(&mut app, "marketplace")
    }

    pub fn run_direct_hook_acceptance() -> Result<(), String> {
        let mut arguments = std::env::args_os().skip(2);
        let app_binary = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| "missing packaged app binary path".to_owned())?;
        let hook_binary = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| "missing hook binary path".to_owned())?;
        let app_bundle = arguments
            .next()
            .map(PathBuf::from)
            .ok_or_else(|| "missing packaged app bundle path".to_owned())?;
        if arguments.next().is_some()
            || !app_binary.is_file()
            || !hook_binary.is_file()
            || !app_bundle.is_dir()
            || app_bundle.extension().and_then(|value| value.to_str()) != Some("app")
            || !app_binary
                .canonicalize()
                .map_err(|error| format!("packaged app binary could not be resolved: {error}"))?
                .starts_with(
                    app_bundle
                        .canonicalize()
                        .map_err(|error| format!("app bundle could not be resolved: {error}"))?,
                )
        {
            return Err("acceptance binary paths are invalid".to_owned());
        }

        let workspace = AcceptanceWorkspace::new()?;
        workspace.write_action_config()?;
        let mut app = spawn_app(&app_binary, workspace.path(), workspace.home(), true)?;
        let credential_path = workspace.application_paths().credentials_path();
        if !wait_for_file(&credential_path, Duration::from_secs(30)) {
            terminate(&mut app);
            return Err("packaged app did not publish forwarding credentials".to_owned());
        }
        if let Err(error) = invoke_direct_hook(&hook_binary, &workspace) {
            terminate(&mut app);
            return Err(error);
        }
        wait_for_completion(&mut app, "direct-hook")
    }

    fn invoke_direct_hook(
        hook_binary: &Path,
        workspace: &AcceptanceWorkspace,
    ) -> Result<(), String> {
        let mut child = Command::new(hook_binary)
            .args(["--plugin-hook", "--json-stdin"])
            .env(
                "PLUGIN_DATA",
                workspace
                    .path()
                    .join("plugins")
                    .join("data")
                    .join("lili-lili-local"),
            )
            .env("LILI_PLUGIN_CODEX_HOME", workspace.path())
            .env("HOME", workspace.home())
            .env("XDG_STATE_HOME", workspace.home().join("state"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("direct packaged hook could not start: {error}"))?;
        child
            .stdin
            .take()
            .expect("piped hook stdin is available")
            .write_all(PAYLOAD)
            .map_err(|error| format!("direct packaged hook input failed: {error}"))?;
        let output = child
            .wait_with_output()
            .map_err(|error| format!("direct packaged hook wait failed: {error}"))?;
        if !output.status.success() || !output.stdout.is_empty() || !output.stderr.is_empty() {
            return Err("direct packaged hook did not complete silently".to_owned());
        }
        Ok(())
    }

    fn wait_for_completion(app: &mut Child, delivery: &str) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match app.try_wait() {
                Ok(Some(status)) if status.success() => {
                    println!(
                        "{{\"macosAcceptance\":\"passed\",\"delivery\":{delivery:?},\"target\":\"{}\"}}",
                        MACOS_ARM64.triple,
                    );
                    return Ok(());
                }
                Ok(Some(status)) => {
                    return Err(format!("packaged app exited with {status}"));
                }
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(50));
                }
                Ok(None) => {
                    terminate(app);
                    return Err("packaged app did not quit cleanly".to_owned());
                }
                Err(error) => return Err(format!("packaged app could not be observed: {error}")),
            }
        }
    }

    fn spawn_app(
        binary: &Path,
        codex_home: &Path,
        application_home: &Path,
        action_only: bool,
    ) -> Result<Child, String> {
        let mut command = Command::new(binary);
        command
            .arg(if action_only {
                "--notification-action-acceptance"
            } else {
                "--desktop-acceptance"
            })
            .env("CODEX_HOME", codex_home)
            .env("HOME", application_home)
            .env("XDG_STATE_HOME", application_home.join("state"));
        command
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("packaged app could not start: {error}"))
    }

    fn wait_for_file(path: &Path, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if path.is_file() {
                return true;
            }
            thread::sleep(Duration::from_millis(25));
        }
        false
    }

    fn terminate(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    struct AcceptanceWorkspace {
        path: PathBuf,
        home: PathBuf,
    }

    impl AcceptanceWorkspace {
        fn new() -> Result<Self, String> {
            let path = PathBuf::from("/tmp").join(format!(
                "lili-macos-acceptance-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|error| error.to_string())?
                    .as_nanos()
            ));
            let home = PathBuf::from(format!("/tmp/lm-{}", std::process::id()));
            fs::create_dir_all(&path)
                .map_err(|error| format!("acceptance workspace could not be created: {error}"))?;
            fs::create_dir_all(&home)
                .map_err(|error| format!("application home could not be created: {error}"))?;
            Ok(Self { path, home })
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn home(&self) -> &Path {
            &self.home
        }

        fn application_paths(&self) -> ApplicationPaths {
            ApplicationPaths::from_root(
                self.home
                    .join("Library")
                    .join("Application Support")
                    .join(lili_storage::APPLICATION_IDENTIFIER),
            )
            .expect("acceptance application path must be absolute")
        }

        fn action_context_path(&self) -> PathBuf {
            self.application_paths()
                .root()
                .join("desktop-acceptance-action-context.json")
        }

        fn write_action_config(&self) -> Result<(), String> {
            let application_paths = self.application_paths();
            fs::create_dir_all(application_paths.config_root()).map_err(|error| {
                format!("application config directory could not be created: {error}")
            })?;
            let fixture = std::env::current_exe()
                .map_err(|error| format!("action fixture path could not be resolved: {error}"))?;
            let source = format!(
                r#"version = 1

[[action]]
id = "open-session-context"
trigger = "notification_activate"
command = [{}, "--record-action", {}]

[[action]]
id = "macos-timeout"
trigger = "notification_activate"
command = ["/bin/sleep", "5"]
timeout_ms = 100

[[action]]
id = "acceptance-title"
trigger = "session_title"
command = [{}, "--title-action"]
"#,
                toml_string(&fixture)?,
                toml_string(&self.action_context_path())?,
                toml_string(&fixture)?,
            );
            fs::write(application_paths.actions_path(), source)
                .map_err(|error| format!("acceptance action config could not be written: {error}"))
        }
    }

    fn toml_string(path: &Path) -> Result<String, String> {
        serde_json::to_string(
            path.to_str()
                .ok_or_else(|| "acceptance path is not UTF-8".to_owned())?,
        )
        .map_err(|error| format!("acceptance path could not be encoded: {error}"))
    }

    impl Drop for AcceptanceWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
            let _ = fs::remove_dir_all(&self.home);
        }
    }
}
