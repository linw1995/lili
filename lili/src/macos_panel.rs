use std::{ffi::CStr, sync::OnceLock};

use objc2::{
    msg_send,
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel},
};
use objc2_app_kit::NSWindowCollectionBehavior;

const PANEL_CLASS_NAME: &CStr = c"LiliPetPanel";

pub fn configure(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    let raw_window = window.ns_window()?;
    if raw_window.is_null() {
        return Err(tauri::Error::InvalidWindowHandle);
    }
    let native_window = unsafe { &*raw_window.cast::<AnyObject>() };
    let panel_class = panel_class();
    let old_class = native_window.class();
    assert_eq!(
        old_class.instance_size(),
        panel_class.instance_size(),
        "native panel conversion requires ABI-compatible window classes"
    );
    unsafe {
        objc2::ffi::object_setClass(native_window as *const _ as *mut _, panel_class);
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
    Ok(())
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
    let actual = u8::from(is_panel)
        | (u8::from(is_floating) << 1)
        | (u8::from(!hides_on_deactivate) << 2)
        | (u8::from(behavior.contains(NSWindowCollectionBehavior::CanJoinAllSpaces)) << 3)
        | (u8::from(behavior.contains(NSWindowCollectionBehavior::Stationary)) << 4)
        | (u8::from(behavior.contains(NSWindowCollectionBehavior::IgnoresCycle)) << 5);
    actual == 0b11_1111
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
