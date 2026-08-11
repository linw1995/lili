use std::{
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::Deserialize;
use tauri::{AppHandle, WebviewWindow};

pub const SCRIPT: &str = r#"
window.addEventListener('DOMContentLoaded', () => {
  const startedAt = Date.now();
  let activated = false;
  const finish = (report) => {
    window.__TAURI_INTERNALS__.invoke('complete_desktop_acceptance', { report });
  };
  const poll = window.setInterval(() => {
    const pet = document.querySelector('.pet-atlas');
    const imageReady = pet instanceof HTMLImageElement
      && pet.complete
      && pet.naturalWidth === 1536
      && pet.naturalHeight === 2288
      && pet.closest('.pet-sprite')?.getAttribute('data-tauri-drag-region') === 'deep';
    const transparent = getComputedStyle(document.body).backgroundColor
      .replaceAll(' ', '') === 'rgba(0,0,0,0)';
    const notification = document.querySelector('.notification-activate');
    if (!activated && notification instanceof HTMLButtonElement) {
      activated = true;
      notification.click();
    }
    const feedback = document.querySelector('.action-feedback[data-action-result="failure"]');
    const actionTimedOut = feedback?.textContent?.includes('Action timed out') === true;
    if (imageReady && transparent && activated && actionTimedOut) {
      window.clearInterval(poll);
      finish({
        transparent,
        pinnedContent: imageReady,
        hookDelivered: activated,
        actionTimedOut,
      });
      return;
    }
    if (Date.now() - startedAt > 20_000) {
      window.clearInterval(poll);
      finish({
        transparent,
        pinnedContent: imageReady,
        hookDelivered: activated,
        actionTimedOut,
      });
    }
  }, 50);
}, { once: true });
"#;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAcceptanceReport {
    transparent: bool,
    pinned_content: bool,
    hook_delivered: bool,
    action_timed_out: bool,
}

#[derive(Default)]
pub struct DesktopAcceptanceState {
    codex_home: Mutex<Option<PathBuf>>,
    completed: AtomicBool,
}

impl DesktopAcceptanceState {
    pub fn configure(&self, codex_home: Option<PathBuf>) {
        *self
            .codex_home
            .lock()
            .expect("desktop acceptance state must not be poisoned") = codex_home;
    }
}

#[tauri::command]
pub fn complete_desktop_acceptance(
    app: AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, DesktopAcceptanceState>,
    report: BrowserAcceptanceReport,
) {
    if state.completed.swap(true, Ordering::AcqRel) {
        return;
    }
    let codex_home = state.codex_home.lock().ok().and_then(|path| path.clone());
    let window_contract = window.is_always_on_top().is_ok_and(|enabled| enabled)
        && window.is_decorated().is_ok_and(|decorated| !decorated)
        && crate::current_window_placement(&window).is_some();
    let tray_contract = app.tray_by_id("lili-tray").is_some();
    let visibility_contract = window.hide().is_ok()
        && window.is_visible().is_ok_and(|visible| !visible)
        && window.show().is_ok()
        && window.is_visible().is_ok_and(|visible| visible);
    let transport_contract = codex_home
        .as_deref()
        .is_some_and(private_unix_transport_is_live);
    let passed = cfg!(target_os = "macos")
        && report.transparent
        && report.pinned_content
        && report.hook_delivered
        && report.action_timed_out
        && window_contract
        && tray_contract
        && visibility_contract
        && transport_contract;
    app.exit(if passed { 0 } else { 1 });
}

#[cfg(unix)]
fn private_unix_transport_is_live(codex_home: &std::path::Path) -> bool {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

    let runtime_dir = codex_home.join("lili").join("runtime");
    let current_uid = rustix::process::geteuid().as_raw();
    let Ok(runtime_metadata) = std::fs::symlink_metadata(&runtime_dir) else {
        return false;
    };
    let store = lili_session::ForwardingCredentialStore::for_runtime_dir(&runtime_dir);
    let Ok(record) = store.load() else {
        return false;
    };
    let Some(socket_path) = record.endpoint().unix_path() else {
        return false;
    };
    let Ok(socket_metadata) = std::fs::symlink_metadata(socket_path) else {
        return false;
    };
    runtime_metadata.is_dir()
        && runtime_metadata.uid() == current_uid
        && runtime_metadata.permissions().mode() & 0o077 == 0
        && socket_metadata.file_type().is_socket()
        && socket_metadata.uid() == current_uid
        && socket_metadata.permissions().mode() & 0o077 == 0
}

#[cfg(not(unix))]
fn private_unix_transport_is_live(_codex_home: &std::path::Path) -> bool {
    false
}
