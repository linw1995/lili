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
use objc2::{
    msg_send,
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel},
};
use objc2_app_kit::{
    NSEvent, NSEventMask, NSEventType, NSWindowCollectionBehavior, NSWindowStyleMask,
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

type ContextMenuHandler = Arc<dyn Fn() + Send + Sync>;
type ContextMenuTarget = Arc<AtomicBool>;

#[derive(Default)]
struct ContextMenuState {
    pet: Option<(isize, ContextMenuTarget, ContextMenuHandler)>,
    suppressed_windows: HashSet<isize>,
}

static CONTEXT_MENU_STATE: OnceLock<Mutex<ContextMenuState>> = OnceLock::new();
thread_local! {
    static CONTEXT_MENU_MONITOR: std::cell::RefCell<Option<objc2::rc::Retained<AnyObject>>> =
        const { std::cell::RefCell::new(None) };
}

pub fn configure(
    window: &tauri::WebviewWindow,
    context_menu_target: ContextMenuTarget,
    context_menu_handler: impl Fn() + Send + Sync + 'static,
) -> tauri::Result<()> {
    let window_number = configure_panel(window)?;
    let context_menu_handler: ContextMenuHandler = Arc::new(context_menu_handler);
    let state = CONTEXT_MENU_STATE.get_or_init(|| Mutex::new(ContextMenuState::default()));
    let mut state = state
        .lock()
        .map_err(|_| tauri::Error::AssetNotFound("context menu state".to_owned()))?;
    state.suppressed_windows.insert(window_number);
    state.pet = Some((
        window_number,
        context_menu_target,
        Arc::clone(&context_menu_handler),
    ));
    drop(state);
    install_context_menu_monitor();
    Ok(())
}

pub fn configure_auxiliary(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let window_number = configure_panel(window)?;
    register_context_menu_suppression(window_number)
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
        let Some((suppressed, handler)) = CONTEXT_MENU_STATE.get().and_then(|state| {
            state.lock().ok().map(|state| {
                let suppressed = state.suppressed_windows.contains(&window_number);
                let handler =
                    state
                        .pet
                        .as_ref()
                        .and_then(|(pet_window_number, target, handler)| {
                            (event_type == NSEventType::RightMouseDown
                                && window_number == *pet_window_number
                                && target.load(Ordering::Acquire))
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
            handler();
        }
        std::ptr::null_mut()
    });
    let mask = NSEventMask::RightMouseDown | NSEventMask::RightMouseUp;
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
    if let Some(monitor) = monitor {
        CONTEXT_MENU_MONITOR.with(|slot| *slot.borrow_mut() = Some(monitor));
    }
}
