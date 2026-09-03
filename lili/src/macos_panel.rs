use std::{
    collections::HashSet,
    ffi::CStr,
    ptr::NonNull,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use block2::RcBlock;
use core_graphics::display::CGDisplay;
use objc2::{
    msg_send,
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel},
};
use objc2_app_kit::{
    NSApplication, NSEvent, NSEventMask, NSEventType, NSWindow, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint};

use crate::notification_hit_region::{
    NotificationHitRegionMode, WINDOW_HEIGHT as NOTIFICATION_WINDOW_HEIGHT,
    contains as notification_hit_region_contains,
};

const PANEL_CLASS_NAME: &CStr = c"LiliPetPanel";
const CURRENT_PROCESS: u32 = 2;
const PROCESS_TRANSFORM_TO_UI_ELEMENT_APPLICATION: i32 = 4;

#[repr(C)]
struct ProcessSerialNumber {
    high_long_of_psn: u32,
    low_long_of_psn: u32,
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    #[link_name = "TransformProcessType"]
    fn transform_process_type(
        process_serial_number: *const ProcessSerialNumber,
        transform_state: i32,
    ) -> i32;
}

#[derive(Clone, Copy, Debug)]
pub struct ContextMenuEvent {
    pub screen_x: f64,
    pub screen_y: f64,
    pub timestamp_us: u64,
}

type ContextMenuHandler = Arc<dyn Fn(ContextMenuEvent) + Send + Sync>;

const PET_SPRITE_HEIGHT: f64 = 208.0;
const PET_SPRITE_WIDTH: f64 = 192.0;
const PET_WINDOW_HEIGHT: f64 = 360.0;
const PET_WINDOW_WIDTH: f64 = 320.0;

#[derive(Clone, Copy, Debug)]
struct NotificationHitRegion {
    window_number: isize,
    mode: NotificationHitRegionMode,
    below_pet: bool,
}

#[derive(Default)]
struct ContextMenuState {
    pet: Option<(isize, ContextMenuHandler)>,
    suppressed_windows: HashSet<isize>,
    notification_hit_region: Option<NotificationHitRegion>,
}

static CONTEXT_MENU_STATE: OnceLock<Mutex<ContextMenuState>> = OnceLock::new();
thread_local! {
    static CONTEXT_MENU_MONITOR: std::cell::RefCell<Option<objc2::rc::Retained<AnyObject>>> =
        const { std::cell::RefCell::new(None) };
    static NOTIFICATION_GLOBAL_MONITOR: std::cell::RefCell<
        Option<objc2::rc::Retained<AnyObject>>
    > = const { std::cell::RefCell::new(None) };
}

pub fn configure(
    window: &tauri::WebviewWindow,
    context_menu_handler: impl Fn(ContextMenuEvent) + Send + Sync + 'static,
) -> tauri::Result<()> {
    let window_number = configure_panel(window)?;
    let context_menu_handler: ContextMenuHandler = Arc::new(context_menu_handler);
    let state = CONTEXT_MENU_STATE.get_or_init(|| Mutex::new(ContextMenuState::default()));
    let mut state = state
        .lock()
        .map_err(|_| tauri::Error::AssetNotFound("context menu state".to_owned()))?;
    state.suppressed_windows.insert(window_number);
    state.pet = Some((window_number, Arc::clone(&context_menu_handler)));
    drop(state);
    install_context_menu_monitor();
    Ok(())
}

pub fn configure_auxiliary(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let window_number = configure_panel(window)?;
    register_context_menu_suppression(window_number)?;
    register_notification_hit_region(window_number)
}

pub fn update_notification_hit_region(
    window: &tauri::WebviewWindow,
    mode: NotificationHitRegionMode,
    below_pet: bool,
) -> tauri::Result<()> {
    let state = CONTEXT_MENU_STATE.get_or_init(|| Mutex::new(ContextMenuState::default()));
    let mut state = state
        .lock()
        .map_err(|_| tauri::Error::AssetNotFound("notification hit region".to_owned()))?;
    let hit_region = state
        .notification_hit_region
        .as_mut()
        .ok_or_else(|| tauri::Error::AssetNotFound("notification hit region".to_owned()))?;
    hit_region.mode = mode;
    hit_region.below_pet = below_pet;
    drop(state);
    if MainThreadMarker::new().is_some() {
        refresh_notification_mouse_passthrough();
        Ok(())
    } else {
        window.run_on_main_thread(refresh_notification_mouse_passthrough)
    }
}

pub fn show_without_activation(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let main_thread_window = window.clone();
    window.run_on_main_thread(move || {
        let Ok(raw_window) = main_thread_window.ns_window() else {
            return;
        };
        if raw_window.is_null() {
            return;
        }
        let native_window = unsafe { &*raw_window.cast::<AnyObject>() };
        let sender = std::ptr::null::<AnyObject>();
        unsafe {
            // Tauri's macOS show path calls makeKeyAndOrderFront, which steals keyboard input.
            let _: () = msg_send![native_window, orderFront: sender];
        }
    })
}

pub fn set_position_sync(
    window: &tauri::WebviewWindow,
    position: tauri::PhysicalPosition<i32>,
    destination_scale_factor: f64,
) -> tauri::Result<()> {
    if MainThreadMarker::new().is_some() {
        return set_position_on_main_thread(window, position, destination_scale_factor);
    }

    let target = window.clone();
    let (result_tx, result_rx) = std::sync::mpsc::sync_channel(1);
    window.run_on_main_thread(move || {
        let result = set_position_on_main_thread(&target, position, destination_scale_factor);
        let _ = result_tx.send(result);
    })?;
    result_rx
        .recv()
        .map_err(|_| tauri::Error::AssetNotFound("notification position task".to_owned()))?
}

fn set_position_on_main_thread(
    window: &tauri::WebviewWindow,
    position: tauri::PhysicalPosition<i32>,
    destination_scale_factor: f64,
) -> tauri::Result<()> {
    let raw_window = window.ns_window()?;
    if raw_window.is_null() {
        return Err(tauri::Error::InvalidWindowHandle);
    }
    let native_window = unsafe { &*raw_window.cast::<NSWindow>() };
    let top_left = native_top_left_point(
        position,
        destination_scale_factor,
        CGDisplay::main().pixels_high() as f64,
    );
    // The window still reports its source display scale until this move completes.
    native_window.setFrameTopLeftPoint(top_left);
    Ok(())
}

fn valid_scale_factor(scale_factor: f64) -> f64 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

fn native_top_left_point(
    position: tauri::PhysicalPosition<i32>,
    scale_factor: f64,
    display_height_points: f64,
) -> NSPoint {
    let scale_factor = valid_scale_factor(scale_factor);
    // Match Tao's macOS conversion: the display height is already in AppKit's global
    // coordinate space, while Tao's physical position still needs scale conversion.
    NSPoint::new(
        f64::from(position.x) / scale_factor,
        display_height_points - f64::from(position.y) / scale_factor,
    )
}

fn configure_panel(window: &tauri::WebviewWindow) -> tauri::Result<isize> {
    let raw_window = window.ns_window()?;
    if raw_window.is_null() {
        return Err(tauri::Error::InvalidWindowHandle);
    }
    let native_window = unsafe { &*raw_window.cast::<AnyObject>() };
    let window_number: isize = unsafe { msg_send![native_window, windowNumber] };
    let panel_class = panel_class();
    let old_class = native_window.class();
    assert!(
        old_class.instance_size() >= panel_class.instance_size(),
        "native panel conversion requires enough storage for the panel class"
    );
    unsafe {
        objc2::ffi::object_setClass(native_window as *const _ as *mut _, panel_class);
        let style: NSWindowStyleMask = msg_send![native_window, styleMask];
        let _: () = msg_send![
            native_window,
            setStyleMask: style | NSWindowStyleMask::NonactivatingPanel
        ];
        let _: () = msg_send![native_window, setFloatingPanel: true];
        let _: () = msg_send![native_window, setHidesOnDeactivate: false];
        let _: () = msg_send![native_window, setBecomesKeyOnlyIfNeeded: true];
        let _: () = msg_send![native_window, setWorksWhenModal: true];
        let behavior = NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle
            | NSWindowCollectionBehavior::FullScreenAuxiliary;
        let _: () = msg_send![native_window, setCollectionBehavior: behavior];
    }
    Ok(window_number)
}

pub fn suppress_webview_context_menu(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let raw_window = window.ns_window()?;
    if raw_window.is_null() {
        return Err(tauri::Error::InvalidWindowHandle);
    }
    let native_window = unsafe { &*raw_window.cast::<AnyObject>() };
    let window_number: isize = unsafe { msg_send![native_window, windowNumber] };
    register_context_menu_suppression(window_number)
}

fn register_context_menu_suppression(window_number: isize) -> tauri::Result<()> {
    let state = CONTEXT_MENU_STATE.get_or_init(|| Mutex::new(ContextMenuState::default()));
    state
        .lock()
        .map_err(|_| tauri::Error::AssetNotFound("context menu state".to_owned()))?
        .suppressed_windows
        .insert(window_number);
    install_context_menu_monitor();
    Ok(())
}

fn register_notification_hit_region(window_number: isize) -> tauri::Result<()> {
    let state = CONTEXT_MENU_STATE.get_or_init(|| Mutex::new(ContextMenuState::default()));
    state
        .lock()
        .map_err(|_| tauri::Error::AssetNotFound("notification hit region".to_owned()))?
        .notification_hit_region = Some(NotificationHitRegion {
        window_number,
        mode: NotificationHitRegionMode::Empty,
        below_pet: false,
    });
    install_notification_global_monitor();
    refresh_notification_mouse_passthrough();
    Ok(())
}

fn install_notification_global_monitor() {
    let already_installed = NOTIFICATION_GLOBAL_MONITOR.with(|monitor| monitor.borrow().is_some());
    if already_installed {
        return;
    }
    let mask = NSEventMask::MouseMoved
        | NSEventMask::LeftMouseDragged
        | NSEventMask::RightMouseDragged
        | NSEventMask::OtherMouseDragged;
    let global_block = RcBlock::new(move |_event: NonNull<NSEvent>| {
        refresh_notification_mouse_passthrough();
    });
    if let Some(monitor) =
        NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &global_block)
    {
        NOTIFICATION_GLOBAL_MONITOR.with(|slot| *slot.borrow_mut() = Some(monitor));
    }
}

fn refresh_notification_mouse_passthrough() {
    let tracking_available = CONTEXT_MENU_MONITOR.with(|monitor| monitor.borrow().is_some())
        && NOTIFICATION_GLOBAL_MONITOR.with(|monitor| monitor.borrow().is_some());
    let hit_region = CONTEXT_MENU_STATE
        .get()
        .and_then(|state| state.lock().ok())
        .and_then(|state| state.notification_hit_region);
    let (Some(hit_region), Some(mtm)) = (hit_region, MainThreadMarker::new()) else {
        return;
    };
    let application = NSApplication::sharedApplication(mtm);
    let Some(window) = application.windowWithWindowNumber(hit_region.window_number) else {
        return;
    };
    window.setAcceptsMouseMovedEvents(true);
    if !tracking_available {
        window.setIgnoresMouseEvents(false);
        return;
    }
    let point = window.convertPointFromScreen(NSEvent::mouseLocation());
    window.setIgnoresMouseEvents(!notification_hit_region_contains(
        hit_region.mode,
        hit_region.below_pet,
        point.x,
        NOTIFICATION_WINDOW_HEIGHT - point.y,
    ));
}

pub fn hide_dock_icon() {
    let process_serial_number = ProcessSerialNumber {
        high_long_of_psn: 0,
        low_long_of_psn: CURRENT_PROCESS,
    };
    unsafe {
        let _ = transform_process_type(
            &process_serial_number,
            PROCESS_TRANSFORM_TO_UI_ELEMENT_APPLICATION,
        );
    }
}

pub fn satisfies_desktop_companion_contract(window: &tauri::WebviewWindow) -> bool {
    let Ok(raw_window) = window.ns_window() else {
        return false;
    };
    if raw_window.is_null() {
        return false;
    }
    let native_window = unsafe { &*raw_window.cast::<AnyObject>() };
    let panel_class = AnyClass::get(c"NSPanel").expect("AppKit must provide NSPanel");
    let is_panel: bool = unsafe { msg_send![native_window, isKindOfClass: panel_class] };
    let is_floating: bool = unsafe { msg_send![native_window, isFloatingPanel] };
    let hides_on_deactivate: bool = unsafe { msg_send![native_window, hidesOnDeactivate] };
    let behavior: NSWindowCollectionBehavior =
        unsafe { msg_send![native_window, collectionBehavior] };
    let style: NSWindowStyleMask = unsafe { msg_send![native_window, styleMask] };
    let actual = u16::from(is_panel)
        | (u16::from(is_floating) << 1)
        | (u16::from(!hides_on_deactivate) << 2)
        | (u16::from(style.contains(NSWindowStyleMask::NonactivatingPanel)) << 3)
        | (u16::from(behavior.contains(NSWindowCollectionBehavior::CanJoinAllSpaces)) << 4)
        | (u16::from(behavior.contains(NSWindowCollectionBehavior::Stationary)) << 5)
        | (u16::from(behavior.contains(NSWindowCollectionBehavior::IgnoresCycle)) << 6)
        | (u16::from(application_is_accessory()) << 7)
        | (u16::from(webview_accepts_first_mouse(window)) << 8)
        | (u16::from(webview_suppresses_context_menu(window)) << 9);
    actual == 0x03ff
}

fn webview_accepts_first_mouse(window: &tauri::WebviewWindow) -> bool {
    let accepted = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&accepted);
    let result = window.with_webview(move |webview| {
        let raw_webview = webview.inner();
        if !raw_webview.is_null() {
            let webview = unsafe { &*raw_webview.cast::<AnyObject>() };
            let no_event = std::ptr::null::<AnyObject>();
            let accepts: bool = unsafe { msg_send![webview, acceptsFirstMouse: no_event] };
            observed.store(accepts, Ordering::Release);
        }
    });
    result.is_ok() && accepted.load(Ordering::Acquire)
}

fn webview_suppresses_context_menu(window: &tauri::WebviewWindow) -> bool {
    let Ok(raw_window) = window.ns_window() else {
        return false;
    };
    if raw_window.is_null() {
        return false;
    }
    let native_window = unsafe { &*raw_window.cast::<AnyObject>() };
    let window_number: isize = unsafe { msg_send![native_window, windowNumber] };
    CONTEXT_MENU_STATE
        .get()
        .and_then(|state| state.lock().ok())
        .is_some_and(|state| state.suppressed_windows.contains(&window_number))
}

fn application_is_accessory() -> bool {
    let Some(application_class) = AnyClass::get(c"NSApplication") else {
        return false;
    };
    let application: *mut AnyObject = unsafe { msg_send![application_class, sharedApplication] };
    if application.is_null() {
        return false;
    }
    let activation_policy: isize = unsafe { msg_send![&*application, activationPolicy] };
    activation_policy == 1
}

fn panel_class() -> &'static AnyClass {
    static CLASS: OnceLock<&'static AnyClass> = OnceLock::new();
    CLASS.get_or_init(|| {
        if let Some(existing) = AnyClass::get(PANEL_CLASS_NAME) {
            return existing;
        }
        let superclass = AnyClass::get(c"NSPanel").expect("AppKit must provide NSPanel");
        let mut builder =
            ClassBuilder::new(PANEL_CLASS_NAME, superclass).expect("panel class must be unique");
        unsafe {
            builder.add_method(
                objc2::sel!(canBecomeKeyWindow),
                can_become_key_window as extern "C" fn(_, _) -> _,
            );
        }
        builder.register()
    })
}

extern "C" fn can_become_key_window(_window: &AnyObject, _selector: Sel) -> Bool {
    Bool::YES
}

fn install_context_menu_monitor() {
    let already_installed = CONTEXT_MENU_MONITOR.with(|monitor| monitor.borrow().is_some());
    if already_installed {
        return;
    }
    let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        let event = unsafe { event.as_ref() };
        let window_number = event.windowNumber();
        let event_type = event.r#type();
        if matches!(
            event_type,
            NSEventType::MouseMoved
                | NSEventType::LeftMouseDragged
                | NSEventType::RightMouseDragged
                | NSEventType::OtherMouseDragged
        ) {
            refresh_notification_mouse_passthrough();
            return event as *const NSEvent as *mut NSEvent;
        }
        let Some((suppressed, handler)) = CONTEXT_MENU_STATE.get().and_then(|state| {
            state.lock().ok().map(|state| {
                let suppressed = state.suppressed_windows.contains(&window_number);
                let handler = state.pet.as_ref().and_then(|(pet_window_number, handler)| {
                    should_open_pet_context_menu(
                        event_type,
                        window_number,
                        *pet_window_number,
                        event_hits_pet_sprite(event),
                    )
                    .then(|| Arc::clone(handler))
                });
                (suppressed, handler)
            })
        }) else {
            return event as *const NSEvent as *mut NSEvent;
        };
        if !suppressed {
            return event as *const NSEvent as *mut NSEvent;
        }
        if let Some(handler) = handler {
            let location = NSEvent::mouseLocation();
            handler(ContextMenuEvent {
                screen_x: location.x,
                screen_y: location.y,
                timestamp_us: event_timestamp_us(event.timestamp()),
            });
        }
        std::ptr::null_mut()
    });
    let mask = NSEventMask::RightMouseDown
        | NSEventMask::RightMouseUp
        | NSEventMask::MouseMoved
        | NSEventMask::LeftMouseDragged
        | NSEventMask::RightMouseDragged
        | NSEventMask::OtherMouseDragged;
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
    if let Some(monitor) = monitor {
        CONTEXT_MENU_MONITOR.with(|slot| *slot.borrow_mut() = Some(monitor));
    }
}

pub fn screen_point_to_tauri(
    screen_x: f64,
    screen_y: f64,
    scale_factor: f64,
) -> tauri::PhysicalPosition<i32> {
    screen_point_to_tauri_with_display_height(
        screen_x,
        screen_y,
        scale_factor,
        CGDisplay::main().pixels_high() as f64,
    )
}

fn screen_point_to_tauri_with_display_height(
    screen_x: f64,
    screen_y: f64,
    scale_factor: f64,
    display_height_points: f64,
) -> tauri::PhysicalPosition<i32> {
    let scale_factor = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    // Keep the conversion aligned with Tao's macOS cursor_position implementation.
    let top_left_y = display_height_points - screen_y;
    tauri::PhysicalPosition::new(
        (screen_x * scale_factor).round() as i32,
        (top_left_y * scale_factor).round() as i32,
    )
}

fn event_timestamp_us(timestamp: f64) -> u64 {
    if timestamp.is_finite() && timestamp > 0.0 {
        (timestamp * 1_000_000.0).round() as u64
    } else {
        0
    }
}

fn should_open_pet_context_menu(
    event_type: NSEventType,
    window_number: isize,
    pet_window_number: isize,
    hits_pet_sprite: bool,
) -> bool {
    event_type == NSEventType::RightMouseUp && window_number == pet_window_number && hits_pet_sprite
}

fn event_hits_pet_sprite(event: &NSEvent) -> bool {
    let location = event.locationInWindow();
    pet_sprite_contains(location.x, location.y)
}

fn pet_sprite_contains(x: f64, y: f64) -> bool {
    let min_x = (PET_WINDOW_WIDTH - PET_SPRITE_WIDTH) / 2.0;
    let min_y = (PET_WINDOW_HEIGHT - PET_SPRITE_HEIGHT) / 2.0;
    (min_x..=min_x + PET_SPRITE_WIDTH).contains(&x)
        && (min_y..=min_y + PET_SPRITE_HEIGHT).contains(&y)
}

#[cfg(test)]
mod tests {
    use objc2_app_kit::NSEventType;

    use super::{
        native_top_left_point, pet_sprite_contains, screen_point_to_tauri_with_display_height,
        should_open_pet_context_menu,
    };

    #[test]
    fn pet_hit_region_is_centered_and_includes_its_edges() {
        assert!(pet_sprite_contains(64.0, 76.0));
        assert!(pet_sprite_contains(256.0, 284.0));
        assert!(pet_sprite_contains(160.0, 180.0));
        assert!(!pet_sprite_contains(63.9, 180.0));
        assert!(!pet_sprite_contains(160.0, 284.1));
    }

    #[test]
    fn context_menu_opens_after_the_right_click_position_is_stable() {
        assert!(!should_open_pet_context_menu(
            NSEventType::RightMouseDown,
            7,
            7,
            true
        ));
        assert!(should_open_pet_context_menu(
            NSEventType::RightMouseUp,
            7,
            7,
            true
        ));
        assert!(!should_open_pet_context_menu(
            NSEventType::RightMouseUp,
            7,
            7,
            false
        ));
    }

    #[test]
    fn screen_point_conversion_accounts_for_retina_scale() {
        assert_eq!(
            screen_point_to_tauri_with_display_height(100.0, 450.0, 2.0, 900.0),
            tauri::PhysicalPosition::new(200, 900)
        );
    }

    #[test]
    fn native_frame_position_round_trips_tao_screen_coordinates() {
        let position = screen_point_to_tauri_with_display_height(100.0, 450.0, 2.0, 900.0);
        let top_left = native_top_left_point(position, 2.0, 900.0);
        assert_eq!(top_left.x, 100.0);
        assert_eq!(top_left.y, 450.0);
    }

    #[test]
    fn native_frame_position_uses_the_destination_display_scale() {
        let position = screen_point_to_tauri_with_display_height(100.0, 450.0, 1.0, 900.0);
        let top_left = native_top_left_point(position, 1.0, 900.0);
        assert_eq!(top_left.x, 100.0);
        assert_eq!(top_left.y, 450.0);

        let stale_source_scale = native_top_left_point(position, 2.0, 900.0);
        assert_ne!(stale_source_scale, top_left);
    }
}
