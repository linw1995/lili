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
        fs,
        io::Read,
        path::PathBuf,
        process::{Command, Stdio},
        thread,
        time::Duration,
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
                let input_validated = arguments
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "missing input validation output path".to_owned())?;
                if arguments.next().is_some() {
                    return Err("unexpected fixture arguments".to_owned());
                }
                // The supervisor starts writing only after attaching this parent to its job.
                let mut input = std::io::stdin().lock();
                let mut first_byte = [0];
                input
                    .read_exact(&mut first_byte)
                    .map_err(|error| format!("interaction input could not be read: {error}"))?;
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
                serde_json::Deserializer::from_reader(first_byte.as_slice().chain(input))
                    .into_iter::<serde_json::Value>()
                    .next()
                    .ok_or_else(|| "interaction input is empty".to_owned())?
                    .map_err(|error| format!("interaction input is invalid: {error}"))?;
                fs::write(input_validated, b"")
                    .map_err(|error| format!("input validation could not be recorded: {error}"))?;
                loop {
                    thread::sleep(Duration::from_secs(60));
                }
            }
            _ => Err("invalid fixture mode".to_owned()),
        }
    }
}
