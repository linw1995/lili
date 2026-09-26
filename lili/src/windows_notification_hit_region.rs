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
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    },
};

use crate::appearance_hit_region;
use crate::notification_hit_region::{NotificationHitRegionMode, contains};
use crate::pet_hit_region;

#[derive(Default)]
struct TrackedWindowState {
    window: Option<tauri::WebviewWindow>,
    hwnd: isize,
    scale_factor: f64,
    width: f64,
    height: f64,
    ignores_mouse_events: Option<bool>,
}

#[derive(Default)]
struct NotificationHitRegionState {
    pet: TrackedWindowState,
    appearance: TrackedWindowState,
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
static PET_DRAGGING: AtomicBool = AtomicBool::new(false);
static SETTINGS_DRAGGING: AtomicBool = AtomicBool::new(false);
static LEFT_BUTTON_DOWN: AtomicBool = AtomicBool::new(false);

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

pub(crate) fn configure_pet(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let hwnd = window.hwnd()?;
    let state = NOTIFICATION_HIT_REGION_STATE
        .get_or_init(|| Mutex::new(NotificationHitRegionState::default()));
    let mut state = state
        .lock()
        .map_err(|_| tauri::Error::AssetNotFound("pet hit region".to_owned()))?;
    state.pet.window = Some(window.clone());
    state.pet.hwnd = hwnd.0 as isize;
    state.pet.scale_factor = window
        .scale_factor()
        .ok()
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .unwrap_or(1.0);
    drop(state);
    install_mouse_hook();
    refresh_mouse_passthrough();
    Ok(())
}

pub(crate) fn configure_appearance(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let hwnd = window.hwnd()?;
    let state = NOTIFICATION_HIT_REGION_STATE
        .get_or_init(|| Mutex::new(NotificationHitRegionState::default()));
    let mut state = state
        .lock()
        .map_err(|_| tauri::Error::AssetNotFound("settings hit region".to_owned()))?;
    state.appearance.window = Some(window.clone());
    state.appearance.hwnd = hwnd.0 as isize;
    update_appearance_geometry(&mut state.appearance, window);
    drop(state);
    install_mouse_hook();
    refresh_mouse_passthrough();
    Ok(())
}

pub(crate) fn refresh_appearance(window: &tauri::WebviewWindow) {
    if let Some(state) = NOTIFICATION_HIT_REGION_STATE.get()
        && let Ok(mut state) = state.lock()
    {
        update_appearance_geometry(&mut state.appearance, window);
    }
    refresh_mouse_passthrough();
}

fn update_appearance_geometry(state: &mut TrackedWindowState, window: &tauri::WebviewWindow) {
    let scale = window
        .scale_factor()
        .ok()
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .unwrap_or(1.0);
    state.scale_factor = scale;
    if let Ok(size) = window.inner_size() {
        state.width = f64::from(size.width) / scale;
        state.height = f64::from(size.height) / scale;
    }
}

pub(crate) fn refresh_pet(window: &tauri::WebviewWindow) {
    if let Some(state) = NOTIFICATION_HIT_REGION_STATE.get()
        && let Ok(mut state) = state.lock()
    {
        state.pet.scale_factor = window
            .scale_factor()
            .ok()
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .unwrap_or(1.0);
    }
    refresh_mouse_passthrough();
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
    if code >= 0 {
        handle_mouse_hook_message(wparam.0 as u32);
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn handle_mouse_hook_message(message: u32) {
    match message {
        WM_LBUTTONDOWN => {
            LEFT_BUTTON_DOWN.store(true, Ordering::Release);
            PET_DRAGGING.store(mouse_down_hits_pet(), Ordering::Release);
            SETTINGS_DRAGGING.store(mouse_down_hits_appearance(), Ordering::Release);
            refresh_mouse_passthrough();
        }
        WM_LBUTTONUP => {
            LEFT_BUTTON_DOWN.store(false, Ordering::Release);
            PET_DRAGGING.store(false, Ordering::Release);
            SETTINGS_DRAGGING.store(false, Ordering::Release);
            schedule_appearance_refresh();
        }
        WM_MOUSEMOVE => refresh_mouse_passthrough(),
        _ => {}
    }
}

fn mouse_down_hits_pet() -> bool {
    NOTIFICATION_HIT_REGION_STATE
        .get()
        .and_then(|state| state.lock().ok())
        .and_then(|state| mouse_hits_pet(&state.pet))
        .unwrap_or(false)
}

fn mouse_down_hits_appearance() -> bool {
    NOTIFICATION_HIT_REGION_STATE
        .get()
        .and_then(|state| state.lock().ok())
        .and_then(|state| mouse_hits_appearance(&state.appearance))
        .unwrap_or(false)
}

fn schedule_appearance_refresh() {
    let window = NOTIFICATION_HIT_REGION_STATE
        .get()
        .and_then(|state| state.lock().ok())
        .and_then(|state| state.appearance.window.clone());
    if let Some(window) = window {
        tauri::async_runtime::spawn(async move {
            tokio::task::yield_now().await;
            let _ = window.run_on_main_thread(refresh_mouse_passthrough);
        });
    }
}

fn refresh_mouse_passthrough() {
    let Some((window, ignores_mouse_events)) = next_mouse_passthrough() else {
        refresh_pet_mouse_passthrough();
        refresh_appearance_mouse_passthrough();
        return;
    };
    if window
        .set_ignore_cursor_events(ignores_mouse_events)
        .is_err()
    {
        clear_requested_passthrough();
    }
    refresh_pet_mouse_passthrough();
    refresh_appearance_mouse_passthrough();
}

fn refresh_appearance_mouse_passthrough() {
    let Some((window, ignores_mouse_events)) = next_appearance_mouse_passthrough() else {
        return;
    };
    if window
        .set_ignore_cursor_events(ignores_mouse_events)
        .is_err()
    {
        if let Some(state) = NOTIFICATION_HIT_REGION_STATE.get()
            && let Ok(mut state) = state.lock()
        {
            state.appearance.ignores_mouse_events = None;
        }
    }
}

fn next_appearance_mouse_passthrough() -> Option<(tauri::WebviewWindow, bool)> {
    let state = NOTIFICATION_HIT_REGION_STATE.get()?;
    let mut state = state.lock().ok()?;
    let (window, ignores_mouse_events) = appearance_passthrough_request(&state.appearance)?;
    if state.appearance.ignores_mouse_events == Some(ignores_mouse_events) {
        return None;
    }
    state.appearance.ignores_mouse_events = Some(ignores_mouse_events);
    Some((window, ignores_mouse_events))
}

fn appearance_passthrough_request(
    state: &TrackedWindowState,
) -> Option<(tauri::WebviewWindow, bool)> {
    let window = state.window.clone()?;
    let ignores_mouse_events = requested_appearance_passthrough(state)?;
    Some((window, ignores_mouse_events))
}

fn requested_appearance_passthrough(state: &TrackedWindowState) -> Option<bool> {
    if LEFT_BUTTON_DOWN.load(Ordering::Acquire) && !SETTINGS_DRAGGING.load(Ordering::Acquire) {
        return None;
    }
    Some(
        mouse_hits_appearance(state)
            .is_some_and(|contains| !contains && !SETTINGS_DRAGGING.load(Ordering::Acquire)),
    )
}

fn mouse_hits_appearance(state: &TrackedWindowState) -> Option<bool> {
    if state.width <= 0.0 || state.height <= 0.0 {
        return Some(true);
    }
    let point = cursor_point(state)?;
    Some(appearance_hit_region::contains(
        state.width,
        state.height,
        point.0,
        point.1,
    ))
}

fn refresh_pet_mouse_passthrough() {
    let Some((window, ignores_mouse_events)) = next_pet_mouse_passthrough() else {
        return;
    };
    if window
        .set_ignore_cursor_events(ignores_mouse_events)
        .is_err()
    {
        if let Some(state) = NOTIFICATION_HIT_REGION_STATE.get()
            && let Ok(mut state) = state.lock()
        {
            state.pet.ignores_mouse_events = None;
        }
    }
}

fn next_pet_mouse_passthrough() -> Option<(tauri::WebviewWindow, bool)> {
    let state = NOTIFICATION_HIT_REGION_STATE.get()?;
    let mut state = state.lock().ok()?;
    let window = state.pet.window.clone()?;
    let ignores_mouse_events = mouse_hits_pet(&state.pet)
        .is_some_and(|contains| !contains && !PET_DRAGGING.load(Ordering::Acquire));
    if state.pet.ignores_mouse_events == Some(ignores_mouse_events) {
        return None;
    }
    state.pet.ignores_mouse_events = Some(ignores_mouse_events);
    Some((window, ignores_mouse_events))
}

fn mouse_hits_pet(state: &TrackedWindowState) -> Option<bool> {
    let (x, y) = cursor_point(state)?;
    Some(pet_hit_region::contains(x, y))
}

fn cursor_point(state: &TrackedWindowState) -> Option<(f64, f64)> {
    if !MOUSE_HOOK_INSTALLED.load(Ordering::Acquire) {
        return None;
    }
    let hwnd = HWND(state.hwnd as _);
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.ok()?;
    unsafe { ScreenToClient(hwnd, &mut point) }
        .as_bool()
        .then_some(())?;
    Some((
        f64::from(point.x) / state.scale_factor,
        f64::from(point.y) / state.scale_factor,
    ))
}

fn next_mouse_passthrough() -> Option<(tauri::WebviewWindow, bool)> {
    let state = NOTIFICATION_HIT_REGION_STATE.get()?;
    let mut state = state.lock().ok()?;
    let window = state.window.clone()?;
    let ignores_mouse_events = mouse_hits_window(&state).is_some_and(|contains| !contains);
    if state.ignores_mouse_events == Some(ignores_mouse_events) {
        return None;
    }
    state.ignores_mouse_events = Some(ignores_mouse_events);
    Some((window, ignores_mouse_events))
}

fn mouse_hits_window(state: &NotificationHitRegionState) -> Option<bool> {
    if !MOUSE_HOOK_INSTALLED.load(Ordering::Acquire) {
        return None;
    }
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
}

fn clear_requested_passthrough() {
    if let Some(state) = NOTIFICATION_HIT_REGION_STATE.get()
        && let Ok(mut state) = state.lock()
    {
        state.ignores_mouse_events = None;
    }
}
