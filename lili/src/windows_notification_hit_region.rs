use std::{
    cell::RefCell,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use windows::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM},
    Graphics::Gdi::ScreenToClient,
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{
        CallNextHookEx, GetCursorPos, HHOOK, SetWindowsHookExW, UnhookWindowsHookEx, WH_MOUSE_LL,
        WM_MOUSEMOVE,
    },
};

use crate::notification_hit_region::{NotificationHitRegionMode, contains};

#[derive(Default)]
struct NotificationHitRegionState {
    window: Option<tauri::WebviewWindow>,
    hwnd: isize,
    mode: NotificationHitRegionMode,
    below_pet: bool,
    scale_factor: f64,
    ignores_mouse_events: Option<bool>,
}

struct MouseHook(HHOOK);

impl Drop for MouseHook {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.0);
        }
    }
}

static NOTIFICATION_HIT_REGION_STATE: OnceLock<Mutex<NotificationHitRegionState>> = OnceLock::new();
static MOUSE_HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static MOUSE_HOOK: RefCell<Option<MouseHook>> = const { RefCell::new(None) };
}

pub(crate) fn configure(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let hwnd = window.hwnd()?;
    let state = NOTIFICATION_HIT_REGION_STATE
        .get_or_init(|| Mutex::new(NotificationHitRegionState::default()));
    let mut state = state
        .lock()
        .map_err(|_| tauri::Error::AssetNotFound("notification hit region".to_owned()))?;
    state.window = Some(window.clone());
    state.hwnd = hwnd.0 as isize;
    drop(state);
    install_mouse_hook();
    update(window, NotificationHitRegionMode::Empty, false)
}

pub(crate) fn update(
    window: &tauri::WebviewWindow,
    mode: NotificationHitRegionMode,
    below_pet: bool,
) -> tauri::Result<()> {
    let state = NOTIFICATION_HIT_REGION_STATE
        .get_or_init(|| Mutex::new(NotificationHitRegionState::default()));
    let scale_factor = window
        .scale_factor()
        .ok()
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .unwrap_or(1.0);
    let mut state = state
        .lock()
        .map_err(|_| tauri::Error::AssetNotFound("notification hit region".to_owned()))?;
    state.window = Some(window.clone());
    state.mode = mode;
    state.below_pet = below_pet;
    state.scale_factor = scale_factor;
    drop(state);
    refresh_mouse_passthrough();
    Ok(())
}

fn install_mouse_hook() {
    let already_installed = MOUSE_HOOK.with(|hook| hook.borrow().is_some());
    if already_installed {
        return;
    }
    let module = unsafe { GetModuleHandleW(None) }
        .ok()
        .map(|module| HINSTANCE(module.0));
    let Ok(hook) =
        (unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(notification_mouse_hook), module, 0) })
    else {
        return;
    };
    MOUSE_HOOK.with(|slot| *slot.borrow_mut() = Some(MouseHook(hook)));
    MOUSE_HOOK_INSTALLED.store(true, Ordering::Release);
}

unsafe extern "system" fn notification_mouse_hook(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // The hook only wakes hit testing; it never suppresses or records system mouse input.
    if code >= 0 && wparam.0 as u32 == WM_MOUSEMOVE {
        refresh_mouse_passthrough();
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn refresh_mouse_passthrough() {
    let Some(state) = NOTIFICATION_HIT_REGION_STATE.get() else {
        return;
    };
    let Ok(mut state) = state.lock() else {
        return;
    };
    let Some(window) = state.window.clone() else {
        return;
    };
    let hit_test = if MOUSE_HOOK_INSTALLED.load(Ordering::Acquire) {
        (|| {
            let hwnd = HWND(state.hwnd as _);
            let mut point = POINT::default();
            unsafe { GetCursorPos(&mut point) }.ok()?;
            unsafe { ScreenToClient(hwnd, &mut point) }
                .as_bool()
                .then_some(())?;
            let scale = state.scale_factor;
            Some(contains(
                state.mode,
                state.below_pet,
                f64::from(point.x) / scale,
                f64::from(point.y) / scale,
            ))
        })()
    } else {
        None
    };
    let ignores_mouse_events = hit_test.is_some_and(|contains| !contains);
    if state.ignores_mouse_events == Some(ignores_mouse_events) {
        return;
    }
    state.ignores_mouse_events = Some(ignores_mouse_events);
    drop(state);
    if window
        .set_ignore_cursor_events(ignores_mouse_events)
        .is_err()
        && let Some(state) = NOTIFICATION_HIT_REGION_STATE.get()
        && let Ok(mut state) = state.lock()
    {
        state.ignores_mouse_events = None;
    }
}
