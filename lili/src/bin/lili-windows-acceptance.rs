#[cfg(target_os = "windows")]
fn main() {
    if let Err(error) = windows::run() {
        eprintln!("Windows acceptance failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("Windows acceptance requires Windows");
    std::process::exit(2);
}

#[cfg(target_os = "windows")]
mod windows {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Storage::FileSystem::SYNCHRONIZE,
        System::Threading::{OpenProcess, WaitForSingleObject},
    };

    const PAYLOAD: &str = r#"{"version":1,"provider":"codex","type":"attention_required","eventId":"windows-acceptance-event","sessionId":"windows-acceptance-session","turnId":"windows-acceptance-turn","occurredAtMs":1800000000000,"project":{"label":"Acceptance"},"summary":"Interaction required"}"#;

    pub fn run() -> Result<(), String> {
        let mut arguments = std::env::args_os().skip(1);
        let app_binary = required_file(arguments.next(), "packaged app")?;
        let hook_binary = required_file(arguments.next(), "hook")?;
        let action_fixture = required_file(arguments.next(), "action fixture")?;
        let action_fixture = fs::canonicalize(action_fixture)
            .map_err(|error| format!("action fixture path could not be resolved: {error}"))?;
        let installer = required_file(arguments.next(), "NSIS installer")?;
        if arguments.next().is_some()
            || installer.extension().and_then(|value| value.to_str()) != Some("exe")
        {
            return Err("acceptance binary paths are invalid".to_owned());
        }

        let workspace = AcceptanceWorkspace::new(action_fixture.clone())?;
        probe_action_fixture(&action_fixture, workspace.fixture_probe())?;
        let mut app = spawn_app(&app_binary, workspace.path())?;
        let credential_path = workspace
            .path()
            .join("lili")
            .join("runtime")
            .join("forwarding.json");
        if !wait_for_file(&credential_path, Duration::from_secs(20)) {
            terminate(&mut app);
            return Err("packaged app did not publish forwarding credentials".to_owned());
        }
        let hook = Command::new(hook_binary)
            .args(["--json-argv", PAYLOAD])
            .env("CODEX_HOME", workspace.path())
            .output()
            .map_err(|error| format!("hook forwarder could not start: {error}"))?;
        if !hook.status.success() || !hook.stdout.is_empty() || !hook.stderr.is_empty() {
            terminate(&mut app);
            return Err("hook delivery did not complete silently".to_owned());
        }
        wait_for_clean_exit(&mut app, Duration::from_secs(35))
            .map_err(|error| workspace.with_fixture_status(&error))?;
        let process_ids = wait_for_process_ids(workspace.process_ids(), Duration::from_secs(5))
            .map_err(|error| workspace.with_fixture_status(&error))?;
        if process_ids.len() != 2 {
            return Err(workspace.with_fixture_status(
                "action fixture did not record exactly one parent and one child",
            ));
        }
        if process_ids.into_iter().any(process_is_alive) {
            return Err(workspace.with_fixture_status("timed-out action left a process tree alive"));
        }
        println!("{{\"windowsAcceptance\":\"passed\"}}");
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

    fn spawn_app(binary: &Path, codex_home: &Path) -> Result<Child, String> {
        Command::new(binary)
            .arg("--desktop-acceptance")
            .env("CODEX_HOME", codex_home)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("packaged app could not start: {error}"))
    }

    fn probe_action_fixture(binary: &Path, output: &Path) -> Result<(), String> {
        let started = Instant::now();
        let mut fixture = Command::new(binary)
            .arg("--probe")
            .arg(output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("action fixture startup probe could not start: {error}"))?;
        wait_for_exit(
            &mut fixture,
            Duration::from_secs(30),
            "action fixture startup probe",
        )?;
        let probe = fs::read_to_string(output).map_err(|error| {
            format!("action fixture startup probe did not report ready: {error}")
        })?;
        if probe != "ready\n" {
            return Err("action fixture startup probe output is invalid".to_owned());
        }
        println!(
            "{{\"actionFixtureProbeMs\":{}}}",
            started.elapsed().as_millis()
        );
        Ok(())
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
        wait_for_exit(child, timeout, "packaged app")
    }

    fn wait_for_exit(child: &mut Child, timeout: Duration, label: &str) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(()),
                Ok(Some(status)) => return Err(format!("{label} exited with {status}")),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                Ok(None) => {
                    terminate(child);
                    return Err(format!("{label} did not quit cleanly"));
                }
                Err(error) => return Err(format!("{label} could not be observed: {error}")),
            }
        }
    }

    fn wait_for_process_ids(path: &Path, timeout: Duration) -> Result<Vec<u32>, String> {
        if !wait_for_file(path, timeout) {
            return Err("action fixture did not record its process tree".to_owned());
        }
        fs::read_to_string(path)
            .map_err(|error| format!("process ids could not be read: {error}"))?
            .lines()
            .map(|line| {
                line.parse::<u32>()
                    .map_err(|_| "process id output is invalid".to_owned())
            })
            .collect()
    }

    fn process_is_alive(process_id: u32) -> bool {
        let process = unsafe { OpenProcess(SYNCHRONIZE, 0, process_id) };
        if process.is_null() {
            return false;
        }
        let wait = unsafe { WaitForSingleObject(process, 0) };
        unsafe {
            CloseHandle(process);
        }
        match wait {
            WAIT_OBJECT_0 => false,
            WAIT_TIMEOUT => true,
            _ => true,
        }
    }

    fn terminate(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    struct AcceptanceWorkspace {
        path: PathBuf,
        process_ids: PathBuf,
        fixture_status: PathBuf,
        fixture_probe: PathBuf,
    }

    impl AcceptanceWorkspace {
        fn new(action_fixture: PathBuf) -> Result<Self, String> {
            let path = std::env::temp_dir().join(format!(
                "lili-windows-acceptance-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|error| error.to_string())?
                    .as_nanos()
            ));
            fs::create_dir_all(path.join("lili"))
                .map_err(|error| format!("acceptance workspace could not be created: {error}"))?;
            let process_ids = path.join("action-processes.txt");
            let fixture_status = path.join("action-fixture-status.txt");
            let fixture_probe = path.join("action-fixture-probe.txt");
            let command = toml_string(&action_fixture);
            let output = toml_string(&process_ids);
            let status = toml_string(&fixture_status);
            fs::write(
                path.join("lili").join("actions.toml"),
                format!(
                    "version = 1\n\n[[action]]\nid = \"windows-tree-timeout\"\ntrigger = \"notification_activate\"\ncommand = [{command}, \"--parent\", {output}, {status}]\ntimeout_ms = 5000\n"
                ),
            )
            .map_err(|error| format!("acceptance action config could not be written: {error}"))?;
            Ok(Self {
                path,
                process_ids,
                fixture_status,
                fixture_probe,
            })
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn process_ids(&self) -> &Path {
            &self.process_ids
        }

        fn fixture_probe(&self) -> &Path {
            &self.fixture_probe
        }

        fn with_fixture_status(&self, error: &str) -> String {
            let status = fs::read_to_string(&self.fixture_status)
                .map(|status| status.lines().collect::<Vec<_>>().join(" | "))
                .unwrap_or_else(|status_error| format!("unavailable ({status_error})"));
            format!("{error}; fixture status: {status}")
        }
    }

    impl Drop for AcceptanceWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn toml_string(path: &Path) -> String {
        format!("{path:?}")
    }
}
