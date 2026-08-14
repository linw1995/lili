#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) = macos::run() {
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
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    use lili_lib::acceptance_marketplace::{
        MACOS_ARM64, install_local_marketplace_plugin, invoke_installed_plugin_hook,
    };

    const PAYLOAD: &[u8] = include_bytes!(
        "../../../lili-session/tests/fixtures/codex/0.147.0/permission-request.json"
    );

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
        let mut app = spawn_app(&app_binary, workspace.path())?;
        let credential_path = workspace
            .path()
            .join("lili")
            .join("runtime")
            .join("forwarding.json");
        if !wait_for_file(&credential_path, Duration::from_secs(15)) {
            terminate(&mut app);
            return Err("packaged app did not publish forwarding credentials".to_owned());
        }

        if let Err(error) = invoke_installed_plugin_hook(
            &plugin,
            workspace.path(),
            PAYLOAD,
            &codex_binary,
            &repository_root,
        ) {
            terminate(&mut app);
            return Err(error);
        }

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match app.try_wait() {
                Ok(Some(status)) if status.success() => {
                    println!(
                        "{{\"macosAcceptance\":\"passed\",\"marketplace\":\"lili-local\",\"target\":\"{}\"}}",
                        MACOS_ARM64.triple
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
                    terminate(&mut app);
                    return Err("packaged app did not quit cleanly".to_owned());
                }
                Err(error) => return Err(format!("packaged app could not be observed: {error}")),
            }
        }
    }

    fn spawn_app(binary: &Path, codex_home: &Path) -> Result<Child, String> {
        Command::new(binary)
            .arg("--desktop-acceptance")
            .env("CODEX_HOME", codex_home)
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

    struct AcceptanceWorkspace(PathBuf);

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
            fs::create_dir_all(path.join("lili"))
                .map_err(|error| format!("acceptance workspace could not be created: {error}"))?;
            Ok(Self(path))
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write_action_config(&self) -> Result<(), String> {
            fs::write(
                self.0.join("lili").join("actions.toml"),
                r#"version = 1

[[action]]
id = "macos-timeout"
trigger = "notification_activate"
command = ["/bin/sleep", "5"]
timeout_ms = 100
"#,
            )
            .map_err(|error| format!("acceptance action config could not be written: {error}"))
        }
    }

    impl Drop for AcceptanceWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
