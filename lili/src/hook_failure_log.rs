use std::{
    fs::OpenOptions,
    io::{self, Write},
    time::{SystemTime, UNIX_EPOCH},
};

use lili_storage::ApplicationPaths;

use crate::hook_forwarder::HookResult;

const MAX_LOG_BYTES: u64 = 128 * 1024;

pub fn record(result: &HookResult) -> io::Result<()> {
    let Some(reason) = result.diagnostic.as_deref() else {
        return Ok(());
    };
    let paths = ApplicationPaths::resolve().map_err(io::Error::other)?;
    paths.ensure_layout().map_err(io::Error::other)?;

    let mut options = OpenOptions::new();
    options.create(true).append(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(paths.hook_failures_path())?;
    if file.metadata()?.len() >= MAX_LOG_BYTES {
        file.set_len(0)?;
    }

    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let entry = serde_json::json!({
        "timestampMs": timestamp_ms,
        "exitCode": result.exit_code.value(),
        "hookEvent": result.hook_event.unwrap_or("unknown"),
        "reason": reason,
    });
    let mut line = serde_json::to_vec(&entry)?;
    line.push(b'\n');
    file.write_all(&line)
}
