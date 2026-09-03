use gtk::prelude::*;

use crate::notification_hit_region::{NotificationHitRegionMode, rectangles};

pub(crate) fn configure(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    update(window, NotificationHitRegionMode::Empty, false)
}

pub(crate) fn update(
    window: &tauri::WebviewWindow,
    mode: NotificationHitRegionMode,
    below_pet: bool,
) -> tauri::Result<()> {
    let target = window.clone();
    window.run_on_main_thread(move || {
        let Ok(gtk_window) = target.gtk_window() else {
            let _ = target.set_ignore_cursor_events(false);
            return;
        };
        if !gtk_window.is_realized() {
            gtk_window.realize();
        }
        let Some(native_window) = gtk_window.window() else {
            let _ = target.set_ignore_cursor_events(false);
            return;
        };
        let rectangles = rectangles(mode, below_pet)
            .into_iter()
            .flatten()
            .map(|rect| {
                cairo::RectangleInt::new(
                    rect.x.round() as i32,
                    rect.y.round() as i32,
                    rect.width.round() as i32,
                    rect.height.round() as i32,
                )
            })
            .collect::<Vec<_>>();
        let region = cairo::Region::create_rectangles(&rectangles);
        native_window.input_shape_combine_region(&region, 0, 0);
    })
}
