#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};

pub(crate) const WINDOW_WIDTH: f64 = 320.0;
pub(crate) const WINDOW_HEIGHT: f64 = 360.0;
pub(crate) const SPRITE_WIDTH: f64 = lili_pet::CELL_WIDTH as f64;
pub(crate) const SPRITE_HEIGHT: f64 = lili_pet::CELL_HEIGHT as f64;

pub(crate) const SPRITE_X: f64 = (WINDOW_WIDTH - SPRITE_WIDTH) / 2.0;
pub(crate) const SPRITE_Y: f64 = (WINDOW_HEIGHT - SPRITE_HEIGHT) / 2.0;
const ALPHA_THRESHOLD: u8 = 16;

struct AtlasAlpha {
    pixels: Vec<u8>,
}

impl AtlasAlpha {
    fn from_bytes(bytes: &[u8]) -> Result<Self, image::ImageError> {
        let image = image::load_from_memory(bytes)?.to_rgba8();
        if image.dimensions() != (lili_pet::ATLAS_WIDTH, lili_pet::ATLAS_HEIGHT) {
            return Err(image::ImageError::Limits(
                image::error::LimitError::from_kind(image::error::LimitErrorKind::DimensionError),
            ));
        }
        Ok(Self {
            pixels: image.pixels().map(|pixel| pixel.0[3]).collect(),
        })
    }

    fn contains(&self, row: u8, column: u8, x: usize, y: usize) -> bool {
        let atlas_x = usize::from(column) * lili_pet::CELL_WIDTH as usize + x;
        let atlas_y = usize::from(row) * lili_pet::CELL_HEIGHT as usize + y;
        self.pixels[atlas_y * lili_pet::ATLAS_WIDTH as usize + atlas_x] >= ALPHA_THRESHOLD
    }
}

#[derive(Default)]
struct PetHitState {
    asset_id: Option<String>,
    atlas: Option<AtlasAlpha>,
    row: u8,
    column: u8,
}

static HIT_STATE: OnceLock<RwLock<PetHitState>> = OnceLock::new();
#[cfg(target_os = "linux")]
static DRAGGING: AtomicBool = AtomicBool::new(false);

fn state() -> &'static RwLock<PetHitState> {
    HIT_STATE.get_or_init(|| RwLock::new(PetHitState::default()))
}

pub(crate) fn set_frame(asset_id: &str, row: u8, column: u8) -> bool {
    let Ok(mut state) = state().write() else {
        return false;
    };
    if state.asset_id.as_deref() != Some(asset_id) {
        state.asset_id = Some(asset_id.to_owned());
        state.atlas = None;
    }
    state.row = row;
    state.column = column;
    state.atlas.is_none()
}

pub(crate) fn install_atlas(asset_id: &str, bytes: &[u8]) -> Result<bool, image::ImageError> {
    let atlas = AtlasAlpha::from_bytes(bytes)?;
    let Ok(mut state) = state().write() else {
        return Ok(false);
    };
    if state.asset_id.as_deref() != Some(asset_id) {
        return Ok(false);
    }
    state.atlas = Some(atlas);
    Ok(true)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn contains(x: f64, y: f64) -> bool {
    let Some((x, y)) = sprite_point(x, y) else {
        return false;
    };
    let Ok(state) = state().read() else {
        return true;
    };
    state
        .atlas
        .as_ref()
        .is_none_or(|atlas| atlas.contains(state.row, state.column, x, y))
}

#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn sprite_point(x: f64, y: f64) -> Option<(usize, usize)> {
    let x = x - SPRITE_X;
    let y = y - SPRITE_Y;
    if (0.0..SPRITE_WIDTH).contains(&x) && (0.0..SPRITE_HEIGHT).contains(&y) {
        Some((x as usize, y as usize))
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn rectangles() -> Vec<cairo::RectangleInt> {
    if DRAGGING.load(Ordering::Acquire) {
        return vec![cairo::RectangleInt::new(
            0,
            0,
            WINDOW_WIDTH as i32,
            WINDOW_HEIGHT as i32,
        )];
    }
    let Ok(state) = state().read() else {
        return vec![sprite_rectangle()];
    };
    let Some(atlas) = state.atlas.as_ref() else {
        return vec![sprite_rectangle()];
    };
    frame_rectangles(atlas, state.row, state.column)
}

#[cfg(target_os = "linux")]
fn frame_rectangles(atlas: &AtlasAlpha, row: u8, column: u8) -> Vec<cairo::RectangleInt> {
    let mut rectangles = Vec::new();
    for y in 0..lili_pet::CELL_HEIGHT as usize {
        append_row_rectangles(&mut rectangles, atlas, row, column, y);
    }
    rectangles
}

#[cfg(target_os = "linux")]
fn append_row_rectangles(
    rectangles: &mut Vec<cairo::RectangleInt>,
    atlas: &AtlasAlpha,
    row: u8,
    column: u8,
    y: usize,
) {
    let mut x = 0;
    while x < lili_pet::CELL_WIDTH as usize {
        if !atlas.contains(row, column, x, y) {
            x += 1;
            continue;
        }
        let start = x;
        while x < lili_pet::CELL_WIDTH as usize && atlas.contains(row, column, x, y) {
            x += 1;
        }
        rectangles.push(cairo::RectangleInt::new(
            SPRITE_X as i32 + start as i32,
            SPRITE_Y as i32 + y as i32,
            (x - start) as i32,
            1,
        ));
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn set_dragging(dragging: bool) {
    DRAGGING.store(dragging, Ordering::Release);
}

#[cfg(target_os = "linux")]
fn sprite_rectangle() -> cairo::RectangleInt {
    cairo::RectangleInt::new(
        SPRITE_X as i32,
        SPRITE_Y as i32,
        SPRITE_WIDTH as i32,
        SPRITE_HEIGHT as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::{ALPHA_THRESHOLD, AtlasAlpha, SPRITE_X, SPRITE_Y, sprite_point};

    #[test]
    fn transparent_margin_does_not_capture_input() {
        assert_eq!(sprite_point(SPRITE_X, SPRITE_Y), Some((0, 0)));
        assert_eq!(sprite_point(160.0, 180.0), Some((96, 104)));
        assert_eq!(sprite_point(0.0, 180.0), None);
        assert_eq!(sprite_point(160.0, 0.0), None);
        assert_eq!(sprite_point(256.0, 180.0), None);
        assert_eq!(sprite_point(160.0, 284.0), None);
    }

    #[test]
    fn hit_test_uses_the_selected_frame_alpha() {
        let mut atlas = AtlasAlpha {
            pixels: vec![0; (lili_pet::ATLAS_WIDTH * lili_pet::ATLAS_HEIGHT) as usize],
        };
        let first = (lili_pet::CELL_HEIGHT + 4) * lili_pet::ATLAS_WIDTH + lili_pet::CELL_WIDTH + 3;
        atlas.pixels[first as usize] = ALPHA_THRESHOLD;
        assert!(atlas.contains(1, 1, 3, 4));
        assert!(!atlas.contains(1, 1, 4, 4));
        assert!(!atlas.contains(0, 1, 3, 4));
    }

    #[test]
    fn fallback_sprite_has_transparent_pixels_inside_its_cell() {
        let atlas = AtlasAlpha::from_bytes(include_bytes!(
            "../../lili-pet/assets/fallback/spritesheet.webp"
        ))
        .unwrap();
        let hit_count = (0..lili_pet::CELL_HEIGHT as usize)
            .flat_map(|y| (0..lili_pet::CELL_WIDTH as usize).map(move |x| (x, y)))
            .filter(|&(x, y)| atlas.contains(0, 0, x, y))
            .count();
        assert!(hit_count > 0);
        assert!(hit_count < (lili_pet::CELL_WIDTH * lili_pet::CELL_HEIGHT) as usize);
    }
}
