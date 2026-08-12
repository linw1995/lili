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
      && pet.closest('.pet-sprite')?.getAttribute('data-hit-region') === 'pet';
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
    drag_state: tauri::State<'_, crate::WindowDragState>,
    report: BrowserAcceptanceReport,
) {
    if state.completed.swap(true, Ordering::AcqRel) {
        return;
    }
    let codex_home = state.codex_home.lock().ok().and_then(|path| path.clone());
    let placement = crate::current_window_placement(&window);
    let window_contract = window.is_always_on_top().is_ok_and(|enabled| enabled)
        && window.is_decorated().is_ok_and(|decorated| !decorated)
        && placement.is_some()
        && native_window_contract(&window);
    let dpi_contract = placement.is_some_and(|placement| placement.scale_milli() >= 500);
    let tray_contract = app.tray_by_id("lili-tray").is_some();
    let visibility_contract = window.hide().is_ok()
        && window.is_visible().is_ok_and(|visible| !visible)
        && window.show().is_ok()
        && window.is_visible().is_ok_and(|visible| visible);
    let transport_contract = codex_home.as_deref().is_some_and(private_transport_is_live);
    let absolute_position_contract = absolute_position_contract(&window, &drag_state);
    let passed = cfg!(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux"
    )) && report.transparent
        && report.pinned_content
        && report.hook_delivered
        && report.action_timed_out
        && window_contract
        && dpi_contract
        && tray_contract
        && visibility_contract
        && transport_contract
        && absolute_position_contract;
    app.exit(if passed { 0 } else { 1 });
}

fn absolute_position_contract(window: &WebviewWindow, state: &crate::WindowDragState) -> bool {
    let Ok(original) = window.outer_position() else {
        return false;
    };
    let Ok(scale) = window.scale_factor() else {
        return false;
    };
    let expected = tauri::PhysicalPosition::new(
        original.x.saturating_add((17.0 * scale).round() as i32),
        original.y.saturating_sub((11.0 * scale).round() as i32),
    );
    let moved = crate::begin_window_drag_from(window, state, 1_000, 1_000).is_ok()
        && crate::move_window_to_from(window, state, 1_017, 989).is_ok()
        && window
            .outer_position()
            .is_ok_and(|position| position == expected);
    let restored = window.set_position(original).is_ok()
        && window
            .outer_position()
            .is_ok_and(|position| position == original);
    moved && restored
}

#[cfg(target_os = "macos")]
fn native_window_contract(window: &WebviewWindow) -> bool {
    crate::macos_panel::satisfies_desktop_companion_contract(window)
}

#[cfg(not(target_os = "macos"))]
fn native_window_contract(_window: &WebviewWindow) -> bool {
    true
}

#[cfg(unix)]
fn private_transport_is_live(codex_home: &std::path::Path) -> bool {
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

#[cfg(windows)]
fn private_transport_is_live(codex_home: &std::path::Path) -> bool {
    let runtime_dir = codex_home.join("lili").join("runtime");
    let store = lili_session::ForwardingCredentialStore::for_runtime_dir(&runtime_dir);
    store
        .load()
        .is_ok_and(|record| lili_session::private_forwarding_endpoint_is_live(record.endpoint()))
}
