#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux::run() {
        eprintln!("Linux acceptance failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Linux acceptance requires Linux");
    std::process::exit(2);
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use lili_lib::acceptance_marketplace::{
        LINUX_X86_64, install_local_marketplace_plugin, invoke_installed_plugin_hook,
    };

    const PAYLOAD: &[u8] = include_bytes!(
        "../../../lili-session/tests/fixtures/codex/0.147.0/permission-request.json"
    );

    pub fn run() -> Result<(), String> {
        let mut arguments = std::env::args_os().skip(1);
        let app_binary = required_file(arguments.next(), "packaged app")?;
        let hook_binary = required_file(arguments.next(), "hook")?;
        let bundle = required_file(arguments.next(), "desktop bundle")?;
        let repository_root = required_directory(arguments.next(), "repository root")?;
        let codex_binary = required_file(arguments.next(), "Codex")?;
        if arguments.next().is_some()
            || bundle.extension().and_then(|value| value.to_str()) != Some("deb")
        {
            return Err("acceptance binary paths are invalid".to_owned());
        }

        let workspace = AcceptanceWorkspace::new()?;
        let plugin = install_local_marketplace_plugin(
            &codex_binary,
            &repository_root,
            workspace.path(),
            &hook_binary,
            LINUX_X86_64,
        )?;
        let mut app = spawn_app(&app_binary, workspace.path())?;
        let credential_path = workspace
            .path()
            .join("lili")
            .join("runtime")
            .join("forwarding.json");
        if !wait_for_file(&credential_path, Duration::from_secs(20)) {
            terminate(&mut app);
            return Err("desktop app did not publish forwarding credentials".to_owned());
        }
        if let Err(error) = invoke_installed_plugin_hook(&plugin, workspace.path(), PAYLOAD) {
            terminate(&mut app);
            return Err(error);
        }
        wait_for_clean_exit(&mut app, Duration::from_secs(35))?;
        println!(
            "{{\"linuxAcceptance\":\"passed\",\"marketplace\":\"lili-local\",\"target\":\"{}\"}}",
            LINUX_X86_64.triple
        );
        Ok(())
    }

    fn required_file(value: Option<std::ffi::OsString>, label: &str) -> Result<PathBuf, String> {
        let path = value
            .map(PathBuf::from)
            .ok_or_else(|| format!("missing {label} path"))?;
        path.is_file()
            .then_some(path)
            .ok_or_else(|| format!("{label} path is not a file"))
    }

    fn required_directory(
        value: Option<std::ffi::OsString>,
        label: &str,
    ) -> Result<PathBuf, String> {
        let path = value
            .map(PathBuf::from)
            .ok_or_else(|| format!("missing {label} path"))?;
        path.is_dir()
            .then_some(path)
            .ok_or_else(|| format!("{label} path is not a directory"))
    }

    fn spawn_app(binary: &Path, codex_home: &Path) -> Result<Child, String> {
        Command::new(binary)
            .arg("--desktop-acceptance")
            .env("CODEX_HOME", codex_home)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("desktop app could not start: {error}"))
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

    fn wait_for_clean_exit(child: &mut Child, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => return Err(format!("desktop app exited with {status}")),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                Ok(None) => {
                    terminate(child);
                    return Err("desktop app did not quit cleanly".to_owned());
                }
                Err(error) => return Err(format!("desktop app could not be observed: {error}")),
            }
        }
    }

    fn terminate(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    struct AcceptanceWorkspace(PathBuf);

    impl AcceptanceWorkspace {
        fn new() -> Result<Self, String> {
            let path = std::env::temp_dir().join(format!(
                "lili-linux-acceptance-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|error| error.to_string())?
                    .as_nanos()
            ));
            fs::create_dir_all(path.join("lili"))
                .map_err(|error| format!("acceptance workspace could not be created: {error}"))?;
            fs::write(
                path.join("lili").join("actions.toml"),
                "version = 1\n\n[[action]]\nid = \"linux-timeout\"\ntrigger = \"notification_activate\"\ncommand = [\"/bin/sleep\", \"5\"]\ntimeout_ms = 100\n",
            )
            .map_err(|error| format!("acceptance action config could not be written: {error}"))?;
            Ok(Self(path))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for AcceptanceWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
