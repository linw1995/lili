use std::sync::atomic::{AtomicBool, Ordering};

use tauri::AppHandle;

pub const SCRIPT: &str = r#"
window.addEventListener('DOMContentLoaded', () => {
  const passed = document.querySelector('[data-ssr-marker="lili-ready"]') !== null;
  window.__TAURI_INTERNALS__.invoke('complete_desktop_smoke', { passed });
}, { once: true });
"#;

#[derive(Default)]
pub struct DesktopSmokeState {
    completed: AtomicBool,
}

#[tauri::command]
pub fn complete_desktop_smoke(
    app: AppHandle,
    state: tauri::State<'_, DesktopSmokeState>,
    passed: bool,
) {
    if !state.completed.swap(true, Ordering::AcqRel) {
        app.exit(if passed { 0 } else { 1 });
    }
}
