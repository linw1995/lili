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
                if arguments.next().is_some() {
                    return Err("unexpected fixture arguments".to_owned());
                }
                let mut input = Vec::new();
                std::io::stdin()
                    .read_to_end(&mut input)
                    .map_err(|error| format!("interaction input could not be read: {error}"))?;
                if input.is_empty() {
                    return Err("interaction input is empty".to_owned());
                }
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
}
