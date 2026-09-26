const HORIZONTAL_INSET: f64 = 48.0;
const TOP_INSET: f64 = 32.0;
const BOTTOM_INSET: f64 = 64.0;
const CORNER_RADIUS: f64 = 20.0;
// Transparent window edges still need to receive native resize gestures.
#[cfg(any(target_os = "macos", target_os = "windows", test))]
const RESIZE_EDGE: f64 = 10.0;
const COMPACT_WIDTH: f64 = 560.0;
const COMPACT_HEIGHT: f64 = 600.0;

#[cfg(any(target_os = "macos", target_os = "windows", test))]
pub(crate) fn contains(width: f64, height: f64, x: f64, y: f64) -> bool {
    if !(0.0..width).contains(&x) || !(0.0..height).contains(&y) {
        return false;
    }
    if compact(width, height) || resize_edge(width, height, x, y) {
        return true;
    }
    frame_contains(width, height, x, y)
}

fn compact(width: f64, height: f64) -> bool {
    width <= COMPACT_WIDTH || height <= COMPACT_HEIGHT
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn resize_edge(width: f64, height: f64, x: f64, y: f64) -> bool {
    x < RESIZE_EDGE || x >= width - RESIZE_EDGE || y < RESIZE_EDGE || y >= height - RESIZE_EDGE
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn frame_contains(width: f64, height: f64, x: f64, y: f64) -> bool {
    let left = HORIZONTAL_INSET;
    let right = width - HORIZONTAL_INSET;
    let top = TOP_INSET;
    let bottom = height - BOTTOM_INSET;
    if !(left..right).contains(&x) || !(top..bottom).contains(&y) {
        return false;
    }
    let dx = x - x.clamp(left + CORNER_RADIUS, right - CORNER_RADIUS);
    let dy = y - y.clamp(top + CORNER_RADIUS, bottom - CORNER_RADIUS);
    dx * dx + dy * dy <= CORNER_RADIUS * CORNER_RADIUS
}

#[cfg(target_os = "linux")]
pub(crate) fn rectangles(width: i32, height: i32, resize_edge: i32) -> Vec<cairo::RectangleInt> {
    if compact(f64::from(width), f64::from(height)) {
        return vec![cairo::RectangleInt::new(0, 0, width, height)];
    }
    let mut rectangles = resize_rectangles(width, height, resize_edge);
    for y in TOP_INSET as i32..height - BOTTOM_INSET as i32 {
        rectangles.push(frame_row_rectangle(width, height, y));
    }
    rectangles
}

#[cfg(target_os = "linux")]
fn resize_rectangles(width: i32, height: i32, edge: i32) -> Vec<cairo::RectangleInt> {
    vec![
        cairo::RectangleInt::new(0, 0, width, edge),
        cairo::RectangleInt::new(0, height - edge, width, edge),
        cairo::RectangleInt::new(0, edge, edge, height - edge * 2),
        cairo::RectangleInt::new(width - edge, edge, edge, height - edge * 2),
    ]
}

#[cfg(target_os = "linux")]
fn frame_row_rectangle(width: i32, height: i32, y: i32) -> cairo::RectangleInt {
    let top = TOP_INSET;
    let bottom = f64::from(height) - BOTTOM_INSET;
    let center_y = f64::from(y) + 0.5;
    let distance = if center_y < top + CORNER_RADIUS {
        top + CORNER_RADIUS - center_y
    } else if center_y > bottom - CORNER_RADIUS {
        center_y - (bottom - CORNER_RADIUS)
    } else {
        0.0
    };
    let inset = CORNER_RADIUS - (CORNER_RADIUS * CORNER_RADIUS - distance * distance).sqrt();
    let left = (HORIZONTAL_INSET + inset).ceil() as i32;
    let right = (f64::from(width) - HORIZONTAL_INSET - inset).floor() as i32;
    cairo::RectangleInt::new(left, y, right - left, 1)
}

#[cfg(test)]
mod tests {
    use super::contains;

    #[test]
    fn full_size_window_passes_input_through_shadow_margins_and_corners() {
        let (width, height) = (1180.0, 900.0);
        assert!(contains(width, height, 500.0, 300.0));
        assert!(contains(width, height, 68.0, 32.0));
        assert!(!contains(width, height, 48.0, 32.0));
        assert!(!contains(width, height, 30.0, 400.0));
        assert!(!contains(width, height, 500.0, 20.0));
        assert!(!contains(width, height, 500.0, 850.0));
        assert!(contains(width, height, 3.0, 400.0));
        assert!(contains(width, height, 500.0, 897.0));
    }

    #[test]
    fn compact_layout_accepts_the_whole_window() {
        assert!(contains(560.0, 900.0, 20.0, 20.0));
        assert!(contains(1180.0, 600.0, 20.0, 20.0));
        assert!(!contains(560.0, 900.0, -1.0, 20.0));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_input_rectangles_follow_the_visible_frame() {
        let regions = super::rectangles(1180, 900, 10);
        let hits = |x, y| {
            regions.iter().any(|region| {
                x >= region.x()
                    && x < region.x() + region.width()
                    && y >= region.y()
                    && y < region.y() + region.height()
            })
        };
        assert!(hits(500, 300));
        assert!(hits(3, 400));
        assert!(!hits(30, 400));
        assert!(!hits(48, 32));
    }
}
