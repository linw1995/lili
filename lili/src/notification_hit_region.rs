use serde::Deserialize;

#[cfg(target_os = "macos")]
pub(crate) const WINDOW_HEIGHT: f64 = 158.0;
pub(crate) const WINDOW_WIDTH: f64 = 320.0;

const STACK_HEIGHT: f64 = 148.0;
const STACK_HORIZONTAL_INSET: f64 = 12.0;
const CARD_HEIGHT: f64 = 58.0;
const CARD_TOP_OFFSET: f64 = 12.0;
const CARD_BOTTOM_OFFSET: f64 = 78.0;
const STACK_TOP_ABOVE: f64 = 6.0;
const STACK_TOP_BELOW: f64 = 4.0;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NotificationHitRegionMode {
    #[default]
    Empty,
    Top,
    Bottom,
    Both,
    Reflow,
    Scroll,
}

impl NotificationHitRegionMode {
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Empty => 0,
            Self::Top => 1,
            Self::Bottom => 2,
            Self::Both => 3,
            Self::Reflow => 4,
            Self::Scroll => 5,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::Top,
            2 => Self::Bottom,
            3 => Self::Both,
            4 => Self::Reflow,
            5 => Self::Scroll,
            _ => Self::Empty,
        }
    }

    pub(crate) const fn fallback_for_count(self, notification_count: usize) -> Self {
        match notification_count {
            0 => Self::Empty,
            1 if matches!(self, Self::Top | Self::Bottom | Self::Both | Self::Reflow) => self,
            1 => Self::Bottom,
            2 => Self::Both,
            _ => Self::Scroll,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NotificationHitRect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

impl NotificationHitRect {
    #[cfg(any(test, target_os = "macos", target_os = "windows"))]
    fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

pub(crate) fn rectangles(
    mode: NotificationHitRegionMode,
    below_pet: bool,
) -> [Option<NotificationHitRect>; 2] {
    let stack_top = if below_pet {
        STACK_TOP_BELOW
    } else {
        STACK_TOP_ABOVE
    };
    let card_width = WINDOW_WIDTH - STACK_HORIZONTAL_INSET * 2.0;
    let card = |offset| NotificationHitRect {
        x: STACK_HORIZONTAL_INSET,
        y: stack_top + offset,
        width: card_width,
        height: CARD_HEIGHT,
    };
    let top = card(if below_pet {
        CARD_BOTTOM_OFFSET
    } else {
        CARD_TOP_OFFSET
    });
    let bottom = card(if below_pet {
        CARD_TOP_OFFSET
    } else {
        CARD_BOTTOM_OFFSET
    });
    match mode {
        NotificationHitRegionMode::Empty => [None, None],
        NotificationHitRegionMode::Top => [Some(top), None],
        NotificationHitRegionMode::Bottom => [Some(bottom), None],
        NotificationHitRegionMode::Both => [Some(top), Some(bottom)],
        NotificationHitRegionMode::Reflow => [
            Some(NotificationHitRect {
                x: STACK_HORIZONTAL_INSET,
                y: stack_top + CARD_TOP_OFFSET,
                width: card_width,
                height: CARD_BOTTOM_OFFSET + CARD_HEIGHT - CARD_TOP_OFFSET,
            }),
            None,
        ],
        NotificationHitRegionMode::Scroll => [
            Some(NotificationHitRect {
                x: STACK_HORIZONTAL_INSET,
                y: stack_top,
                width: card_width,
                height: STACK_HEIGHT,
            }),
            None,
        ],
    }
}

#[cfg(any(test, target_os = "macos", target_os = "windows"))]
pub(crate) fn contains(mode: NotificationHitRegionMode, below_pet: bool, x: f64, y: f64) -> bool {
    rectangles(mode, below_pet)
        .into_iter()
        .flatten()
        .any(|rect| rect.contains(x, y))
}

#[cfg(test)]
mod tests {
    use super::{NotificationHitRegionMode as Mode, contains};

    #[test]
    fn hit_regions_match_visible_cards_and_motion_surfaces() {
        assert!(!contains(Mode::Empty, false, 160.0, 110.0));

        assert!(contains(Mode::Bottom, false, 160.0, 110.0));
        assert!(!contains(Mode::Bottom, false, 160.0, 45.0));
        assert!(contains(Mode::Bottom, true, 160.0, 45.0));
        assert!(!contains(Mode::Bottom, true, 160.0, 110.0));

        assert!(contains(Mode::Top, false, 160.0, 45.0));
        assert!(!contains(Mode::Top, false, 160.0, 110.0));
        assert!(contains(Mode::Top, true, 160.0, 110.0));
        assert!(!contains(Mode::Top, true, 160.0, 45.0));

        assert!(contains(Mode::Both, false, 160.0, 45.0));
        assert!(contains(Mode::Both, false, 160.0, 110.0));
        assert!(!contains(Mode::Both, false, 160.0, 78.0));

        assert!(contains(Mode::Reflow, false, 160.0, 78.0));
        assert!(contains(Mode::Scroll, true, 160.0, 78.0));
        assert!(!contains(Mode::Scroll, false, 8.0, 78.0));
        assert!(!contains(Mode::Scroll, false, 312.0, 78.0));
    }

    #[test]
    fn count_fallback_keeps_ui_reported_single_card_motion() {
        assert_eq!(Mode::Empty.fallback_for_count(1), Mode::Bottom);
        assert_eq!(Mode::Both.fallback_for_count(1), Mode::Both);
        assert_eq!(Mode::Top.fallback_for_count(1), Mode::Top);
        assert_eq!(Mode::Reflow.fallback_for_count(1), Mode::Reflow);
        assert_eq!(Mode::Top.fallback_for_count(0), Mode::Empty);
        assert_eq!(Mode::Bottom.fallback_for_count(2), Mode::Both);
        assert_eq!(Mode::Both.fallback_for_count(3), Mode::Scroll);
    }

    #[test]
    fn wire_modes_match_the_ui_contract() {
        for (value, expected) in [
            ("\"empty\"", Mode::Empty),
            ("\"top\"", Mode::Top),
            ("\"bottom\"", Mode::Bottom),
            ("\"both\"", Mode::Both),
            ("\"reflow\"", Mode::Reflow),
            ("\"scroll\"", Mode::Scroll),
        ] {
            assert_eq!(serde_json::from_str::<Mode>(value).unwrap(), expected);
        }
    }
}
