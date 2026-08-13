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
        collections::BTreeSet,
        ffi::OsString,
        fs, mem,
        os::windows::io::AsRawHandle,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        ptr,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Storage::FileSystem::SYNCHRONIZE,
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
            Threading::{OpenProcess, WaitForSingleObject},
        },
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
        probe_action_fixture_with_minimal_environment(&action_fixture, workspace.fixture_probe())?;
        probe_action_fixture_with_pipes(&action_fixture, workspace.fixture_probe())?;
        probe_action_fixture_in_job(&action_fixture, workspace.fixture_job_probe())?;
        probe_action_fixture_in_tokio_job(&action_fixture, workspace.fixture_job_probe())?;
        let mut app = spawn_app(&app_binary, workspace.path())?;
        let process_observer = ProcessObserver::new(&action_fixture, app.id())?;
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
        let fixture_processes = process_observer.finish()?;
        let process_ids = wait_for_process_ids(workspace.process_ids(), Duration::from_secs(5))
            .map_err(|error| {
                workspace.with_fixture_status(&format!(
                    "{error}; observed fixture processes: {}",
                    format_process_observations(&fixture_processes)
                ))
            })?;
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
            .env("RUST_LOG", "lili=info")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| format!("packaged app could not start: {error}"))
    }

    fn probe_action_fixture(binary: &Path, output: &Path) -> Result<(), String> {
        clear_probe_output(output)?;
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
            "{{\"actionFixtureProbe\":\"inherited\",\"elapsedMs\":{}}}",
            started.elapsed().as_millis()
        );
        Ok(())
    }

    fn probe_action_fixture_with_minimal_environment(
        binary: &Path,
        output: &Path,
    ) -> Result<(), String> {
        clear_probe_output(output)?;
        let started = Instant::now();
        let mut command = Command::new(binary);
        command
            .arg("--probe")
            .arg(output)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        configure_minimal_environment(&mut command)?;
        let mut fixture = command.spawn().map_err(|error| {
            format!("minimal-environment fixture probe could not start: {error}")
        })?;
        wait_for_exit(
            &mut fixture,
            Duration::from_secs(10),
            "minimal-environment fixture probe",
        )?;
        validate_probe_output(output, "minimal-environment fixture probe")?;
        println!(
            "{{\"actionFixtureProbe\":\"minimal-environment\",\"elapsedMs\":{}}}",
            started.elapsed().as_millis()
        );
        Ok(())
    }

    fn probe_action_fixture_with_pipes(binary: &Path, output: &Path) -> Result<(), String> {
        clear_probe_output(output)?;
        let started = Instant::now();
        let mut command = Command::new(binary);
        command
            .arg("--probe")
            .arg(output)
            .current_dir(
                binary
                    .parent()
                    .ok_or_else(|| "action fixture directory is unavailable".to_owned())?,
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_minimal_environment(&mut command)?;
        let mut fixture = command
            .spawn()
            .map_err(|error| format!("piped fixture probe could not start: {error}"))?;
        wait_for_exit(&mut fixture, Duration::from_secs(10), "piped fixture probe")?;
        validate_probe_output(output, "piped fixture probe")?;
        println!(
            "{{\"actionFixtureProbe\":\"piped\",\"elapsedMs\":{}}}",
            started.elapsed().as_millis()
        );
        Ok(())
    }

    fn probe_action_fixture_in_job(binary: &Path, status: &Path) -> Result<(), String> {
        clear_probe_output(status)?;
        let started = Instant::now();
        let mut command = Command::new(binary);
        command
            .arg("--probe-job")
            .arg(status)
            .current_dir(
                binary
                    .parent()
                    .ok_or_else(|| "action fixture directory is unavailable".to_owned())?,
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_minimal_environment(&mut command)?;
        let mut fixture = command
            .spawn()
            .map_err(|error| format!("job fixture probe could not start: {error}"))?;
        let job = create_action_job().inspect_err(|_| terminate(&mut fixture))?;
        let result = run_job_probe(&mut fixture, job, status);
        unsafe {
            CloseHandle(job);
        }
        result?;
        println!(
            "{{\"actionFixtureProbe\":\"job\",\"elapsedMs\":{}}}",
            started.elapsed().as_millis()
        );
        Ok(())
    }

    fn probe_action_fixture_in_tokio_job(binary: &Path, status: &Path) -> Result<(), String> {
        clear_probe_output(status)?;
        let started = Instant::now();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| format!("Tokio fixture probe runtime could not start: {error}"))?;
        runtime.block_on(async {
            let mut command = tokio::process::Command::new(binary);
            command
                .arg("--probe-job")
                .arg(status)
                .current_dir(
                    binary
                        .parent()
                        .ok_or_else(|| "action fixture directory is unavailable".to_owned())?,
                )
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            configure_minimal_environment(command.as_std_mut())?;
            let mut fixture = command
                .spawn()
                .map_err(|error| format!("Tokio fixture probe could not start: {error}"))?;
            let job = create_action_job().inspect_err(|_| {
                let _ = fixture.start_kill();
            })?;
            let process = fixture
                .raw_handle()
                .map(|handle| handle as HANDLE)
                .ok_or_else(|| "Tokio fixture probe process handle is unavailable".to_owned())?;
            if unsafe { AssignProcessToJobObject(job, process) } == 0 {
                let error = std::io::Error::last_os_error();
                let _ = fixture.start_kill();
                unsafe {
                    CloseHandle(job);
                }
                return Err(format!(
                    "Tokio fixture probe could not assign its process: {error}"
                ));
            }
            let wait = tokio::time::timeout(Duration::from_secs(10), fixture.wait()).await;
            unsafe {
                CloseHandle(job);
            }
            let exit = wait
                .map_err(|_| "Tokio fixture probe did not quit cleanly".to_owned())?
                .map_err(|error| format!("Tokio fixture probe could not be observed: {error}"))?;
            if !exit.success() {
                return Err(format!("Tokio fixture probe exited with {exit}"));
            }
            let observations = fs::read_to_string(status)
                .map_err(|error| format!("Tokio fixture probe status is unavailable: {error}"))?;
            if !observations.lines().any(|line| line == "probe-ready") {
                return Err(format!(
                    "Tokio fixture probe did not report ready: {}",
                    observations.lines().collect::<Vec<_>>().join(" | ")
                ));
            }
            Ok::<(), String>(())
        })?;
        println!(
            "{{\"actionFixtureProbe\":\"tokio-job\",\"elapsedMs\":{}}}",
            started.elapsed().as_millis()
        );
        Ok(())
    }

    fn create_action_job() -> Result<HANDLE, String> {
        let job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if job.is_null() {
            return Err(format!(
                "fixture probe could not create a job: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                u32::try_from(mem::size_of_val(&limits)).expect("job limit structure fits in u32"),
            )
        };
        if configured == 0 {
            let error = std::io::Error::last_os_error();
            unsafe {
                CloseHandle(job);
            }
            return Err(format!(
                "fixture probe could not configure its job: {error}"
            ));
        }
        Ok(job)
    }

    fn run_job_probe(fixture: &mut Child, job: HANDLE, status: &Path) -> Result<(), String> {
        if unsafe { AssignProcessToJobObject(job, fixture.as_raw_handle() as HANDLE) } == 0 {
            let error = std::io::Error::last_os_error();
            terminate(fixture);
            return Err(format!(
                "job fixture probe could not assign its process: {error}"
            ));
        }
        wait_for_exit(fixture, Duration::from_secs(10), "job fixture probe")?;
        let observations = fs::read_to_string(status)
            .map_err(|error| format!("job fixture probe status is unavailable: {error}"))?;
        if !observations.lines().any(|line| line == "probe-ready") {
            return Err(format!(
                "job fixture probe did not report ready: {}",
                observations.lines().collect::<Vec<_>>().join(" | ")
            ));
        }
        Ok(())
    }

    fn configure_minimal_environment(command: &mut Command) -> Result<(), String> {
        let system_root = std::env::var_os("SystemRoot")
            .ok_or_else(|| "SystemRoot is unavailable for fixture probe".to_owned())?;
        let mut path = system_root.clone();
        path.push("\\System32");
        command.env_clear().envs([
            (OsString::from("PATH"), path),
            (OsString::from("SystemRoot"), system_root),
        ]);
        Ok(())
    }

    fn clear_probe_output(path: &Path) -> Result<(), String> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "fixture probe output could not be cleared: {error}"
            )),
        }
    }

    fn validate_probe_output(path: &Path, label: &str) -> Result<(), String> {
        let probe = fs::read_to_string(path)
            .map_err(|error| format!("{label} did not report ready: {error}"))?;
        if probe == "ready\n" {
            Ok(())
        } else {
            Err(format!("{label} output is invalid"))
        }
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
                Ok(Some(status)) => return Err(format!("packaged app exited with {status}")),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
                Ok(None) => {
                    terminate(child);
                    return Err("packaged app did not quit cleanly".to_owned());
                }
                Err(error) => {
                    return Err(format!("packaged app could not be observed: {error}"));
                }
            }
        }
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

    type ProcessObservation = (String, u32, u32);

    fn observe_processes(
        executable_name: &str,
        parent_process_id: u32,
    ) -> Result<BTreeSet<ProcessObservation>, String> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(format!(
                "process snapshot could not be created: {}",
                std::io::Error::last_os_error()
            ));
        }
        let result = read_process_snapshot(snapshot, executable_name, parent_process_id);
        unsafe {
            CloseHandle(snapshot);
        }
        result
    }

    fn read_process_snapshot(
        snapshot: HANDLE,
        executable_name: &str,
        parent_process_id: u32,
    ) -> Result<BTreeSet<ProcessObservation>, String> {
        let mut processes = BTreeSet::new();
        let mut process = PROCESSENTRY32W {
            dwSize: u32::try_from(mem::size_of::<PROCESSENTRY32W>())
                .expect("process entry size fits in u32"),
            ..Default::default()
        };
        let mut has_process = unsafe { Process32FirstW(snapshot, &mut process) } != 0;
        while has_process {
            let name_length = process
                .szExeFile
                .iter()
                .position(|character| *character == 0)
                .unwrap_or(process.szExeFile.len());
            let name = String::from_utf16_lossy(&process.szExeFile[..name_length]);
            if name.eq_ignore_ascii_case(executable_name)
                || process.th32ParentProcessID == parent_process_id
            {
                processes.insert((name, process.th32ProcessID, process.th32ParentProcessID));
            }
            has_process = unsafe { Process32NextW(snapshot, &mut process) } != 0;
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(18) {
            Ok(processes)
        } else {
            Err(format!("process snapshot could not be read: {error}"))
        }
    }

    fn format_process_observations(processes: &BTreeSet<ProcessObservation>) -> String {
        if processes.is_empty() {
            return "none".to_owned();
        }
        processes
            .iter()
            .map(|(name, process, parent)| format!("name={name} pid={process} parent={parent}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    struct ProcessObserver {
        stop: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<Result<BTreeSet<ProcessObservation>, String>>>,
    }

    impl ProcessObserver {
        fn new(action_fixture: &Path, parent_process_id: u32) -> Result<Self, String> {
            let executable_name = action_fixture
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| "action fixture file name is invalid".to_owned())?
                .to_owned();
            let stop = Arc::new(AtomicBool::new(false));
            let thread_stop = stop.clone();
            let thread = thread::spawn(move || {
                let mut observations = BTreeSet::new();
                while !thread_stop.load(Ordering::Acquire) {
                    observations.extend(observe_processes(&executable_name, parent_process_id)?);
                    thread::sleep(Duration::from_millis(25));
                }
                Ok(observations)
            });
            Ok(Self {
                stop,
                thread: Some(thread),
            })
        }

        fn finish(mut self) -> Result<BTreeSet<ProcessObservation>, String> {
            self.stop.store(true, Ordering::Release);
            self.thread
                .take()
                .expect("process observer retains its thread")
                .join()
                .map_err(|_| "process observer panicked".to_owned())?
        }
    }

    impl Drop for ProcessObserver {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
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
        fixture_job_probe: PathBuf,
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
            let fixture_job_probe = path.join("action-fixture-job-probe.txt");
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
                fixture_job_probe,
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

        fn fixture_job_probe(&self) -> &Path {
            &self.fixture_job_probe
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
