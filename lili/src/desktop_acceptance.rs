use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use lili_actions::ActionExecutionOutcome;
use lili_app_state::AppState;
use lili_storage::ApplicationPaths;
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
    const hydrated = document.querySelector('#lili-app[data-hydrated="true"]') !== null;
    const notification = document.querySelector('.notification-activate');
    if (!activated && hydrated && notification instanceof HTMLButtonElement) {
      activated = true;
      notification.click();
    }
    const feedback = document.querySelector('.action-feedback[data-action-result="failure"]');
    const actionTimedOut = feedback?.textContent?.includes('Action timed out') === true;
    const feedbackActionId = feedback?.getAttribute('data-action-id') ?? null;
    if (imageReady && transparent && activated && actionTimedOut) {
      window.clearInterval(poll);
      finish({
        transparent,
        pinnedContent: imageReady,
        hydrated,
        hookDelivered: activated,
        actionTimedOut,
        feedbackActionId,
      });
      return;
    }
    if (Date.now() - startedAt > 20_000) {
      window.clearInterval(poll);
      finish({
        transparent,
        pinnedContent: imageReady,
        hydrated,
        hookDelivered: activated,
        actionTimedOut,
        feedbackActionId,
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
    hydrated: bool,
    hook_delivered: bool,
    action_timed_out: bool,
    feedback_action_id: Option<String>,
}

#[derive(Default)]
pub struct DesktopAcceptanceState {
    application_paths: Mutex<Option<ApplicationPaths>>,
    app_state: Mutex<Option<AppState>>,
    completed: AtomicBool,
}

impl DesktopAcceptanceState {
    pub fn configure(&self, application_paths: ApplicationPaths, app_state: AppState) {
        *self
            .application_paths
            .lock()
            .expect("desktop acceptance state must not be poisoned") = Some(application_paths);
        *self
            .app_state
            .lock()
            .expect("desktop acceptance state must not be poisoned") = Some(app_state);
    }
}

#[tauri::command]
pub async fn complete_desktop_acceptance(
    app: AppHandle,
    window: WebviewWindow,
    state: tauri::State<'_, DesktopAcceptanceState>,
    drag_state: tauri::State<'_, crate::WindowDragState>,
    report: BrowserAcceptanceReport,
) -> Result<(), String> {
    if state.completed.swap(true, Ordering::AcqRel) {
        return Ok(());
    }
    let application_paths = state
        .application_paths
        .lock()
        .ok()
        .and_then(|paths| paths.clone());
    let app_state = state.app_state.lock().ok().and_then(|state| state.clone());
    let action_audit = match app_state {
        Some(state) => state.action_audit().await,
        None => Vec::new(),
    };
    let expected_action_id = expected_action_id();
    let action_contract = action_audit.as_slice().first().is_some_and(|entry| {
        action_audit.len() == 1
            && entry.action_id == expected_action_id
            && entry.outcome == ActionExecutionOutcome::TimedOut
    });
    eprintln!(
        "desktop acceptance browser={report:?} audit={}",
        serde_json::to_string(&action_audit).unwrap_or_else(|_| "unavailable".to_owned())
    );
    let placement = crate::current_window_placement(&window);
    let always_on_top_contract = window.is_always_on_top().is_ok_and(|enabled| enabled);
    let undecorated_contract = window.is_decorated().is_ok_and(|decorated| !decorated);
    let placement_contract = placement.is_some();
    let native_window_contract = native_window_contract(&window);
    let window_contract = always_on_top_contract
        && undecorated_contract
        && placement_contract
        && native_window_contract;
    let dpi_contract = placement.is_some_and(|placement| placement.scale_milli() >= 500);
    let tray_contract = app.tray_by_id("lili-tray").is_some();
    let hide_contract = window.hide().is_ok();
    let hidden_contract = hide_contract && window.is_visible().is_ok_and(|visible| !visible);
    let show_contract = hidden_contract && window.show().is_ok();
    let shown_contract = show_contract && window.is_visible().is_ok_and(|visible| visible);
    let visibility_contract = hide_contract && hidden_contract && show_contract && shown_contract;
    let transport_contract = application_paths
        .as_ref()
        .is_some_and(private_transport_is_live);
    let absolute_position_contract = absolute_position_contract(&window, &drag_state);
    eprintln!(
        "desktop acceptance native alwaysOnTop={always_on_top_contract} undecorated={undecorated_contract} placement={placement_contract} nativeWindow={native_window_contract} dpi={dpi_contract} tray={tray_contract} hide={hide_contract} hidden={hidden_contract} show={show_contract} shown={shown_contract} transport={transport_contract} absolutePosition={absolute_position_contract}"
    );
    let passed = cfg!(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux"
    )) && report.transparent
        && report.pinned_content
        && report.hydrated
        && report.hook_delivered
        && report.action_timed_out
        && report.feedback_action_id.as_deref() == Some(expected_action_id)
        && action_contract
        && window_contract
        && dpi_contract
        && tray_contract
        && visibility_contract
        && transport_contract
        && absolute_position_contract;
    let result_recorded = match application_paths {
        Some(application_paths) => std::fs::write(
            application_paths.root().join("desktop-acceptance-result"),
            if passed { "passed\n" } else { "failed\n" },
        )
        .inspect_err(|error| eprintln!("desktop acceptance result could not be recorded: {error}"))
        .is_ok(),
        None => {
            eprintln!(
                "desktop acceptance result could not be recorded: application storage is unavailable"
            );
            false
        }
    };
    app.exit(if passed && result_recorded { 0 } else { 1 });
    Ok(())
}

fn expected_action_id() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows-tree-timeout"
    } else if cfg!(target_os = "macos") {
        "macos-timeout"
    } else {
        "linux-timeout"
    }
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
fn private_transport_is_live(application_paths: &ApplicationPaths) -> bool {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

    let runtime_dir = application_paths.runtime_root();
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
fn private_transport_is_live(application_paths: &ApplicationPaths) -> bool {
    let runtime_dir = application_paths.runtime_root();
    let store = lili_session::ForwardingCredentialStore::for_runtime_dir(&runtime_dir);
    store
        .load()
        .is_ok_and(|record| lili_session::private_forwarding_endpoint_is_live(record.endpoint()))
}
