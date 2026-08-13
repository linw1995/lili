#[cfg(target_os = "windows")]
fn main() {
    if let Err(error) = windows::run() {
        eprintln!("action tree fixture failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("action tree fixture requires Windows");
    std::process::exit(2);
}

#[cfg(target_os = "windows")]
mod windows {
    use std::{
        ffi::c_void,
        fs, mem,
        path::PathBuf,
        process::{Command, Stdio},
        ptr, thread,
        time::{Duration, Instant},
    };

    use windows_sys::Win32::System::{
        JobObjects::{
            JOBOBJECT_BASIC_PROCESS_ID_LIST, JobObjectBasicProcessIdList, QueryInformationJobObject,
        },
        Threading::GetCurrentProcessId,
    };

    pub fn run() -> Result<(), String> {
        let mut arguments = std::env::args_os().skip(1);
        match arguments.next().as_deref() {
            Some(mode) if mode == "--child" => loop {
                thread::sleep(Duration::from_secs(60));
            },
            Some(mode) if mode == "--parent" => {
                let output = arguments
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "missing process id output path".to_owned())?;
                if arguments.next().is_some() {
                    return Err("unexpected fixture arguments".to_owned());
                }
                wait_for_supervisor_job(Duration::from_secs(4))?;
                let child = Command::new(
                    std::env::current_exe()
                        .map_err(|error| format!("fixture path is unavailable: {error}"))?,
                )
                .arg("--child")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| format!("fixture child could not start: {error}"))?;
                fs::write(output, format!("{}\n{}\n", std::process::id(), child.id()))
                    .map_err(|error| format!("process ids could not be written: {error}"))?;
                loop {
                    thread::sleep(Duration::from_secs(60));
                }
            }
            _ => Err("invalid fixture mode".to_owned()),
        }
    }

    fn wait_for_supervisor_job(timeout: Duration) -> Result<(), String> {
        let process_id = unsafe { GetCurrentProcessId() } as usize;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            // A null handle queries the calling process's immediate job. The fresh action job
            // contains only this parent until the fixture creates its child.
            let mut processes = JOBOBJECT_BASIC_PROCESS_ID_LIST::default();
            let queried = unsafe {
                QueryInformationJobObject(
                    ptr::null_mut(),
                    JobObjectBasicProcessIdList,
                    (&raw mut processes).cast::<c_void>(),
                    u32::try_from(mem::size_of_val(&processes))
                        .expect("job process list size fits in u32"),
                    ptr::null_mut(),
                )
            };
            if queried != 0
                && processes.NumberOfAssignedProcesses == 1
                && processes.NumberOfProcessIdsInList == 1
                && processes.ProcessIdList[0] == process_id
            {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(10));
        }
        Err("fixture parent was not isolated in its supervisor job".to_owned())
    }
}
