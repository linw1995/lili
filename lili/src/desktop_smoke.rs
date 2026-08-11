use std::sync::atomic::{AtomicBool, Ordering};

use tauri::AppHandle;

pub const SCRIPT: &str = r#"
window.addEventListener('DOMContentLoaded', () => {
  const pet = document.querySelector('.pet-atlas');
  const finish = (passed) => {
    window.__TAURI_INTERNALS__.invoke('complete_desktop_smoke', { passed });
  };
  const imageIsValid = () => pet instanceof HTMLImageElement
    && pet.complete
    && pet.naturalWidth === 1536
    && pet.naturalHeight === 2288;

  if (imageIsValid()) {
    finish(true);
    return;
  }
  if (!(pet instanceof HTMLImageElement)) {
    finish(false);
    return;
  }

  const timeout = window.setTimeout(() => finish(false), 10_000);
  pet.addEventListener('load', () => {
    window.clearTimeout(timeout);
    finish(imageIsValid());
  }, { once: true });
  pet.addEventListener('error', () => {
    window.clearTimeout(timeout);
    finish(false);
  }, { once: true });
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
