use std::time::Duration;

use thiserror::Error;

use crate::{
    AnimationSpec, AnimationState, FrameDescriptor, LOOK_DIRECTIONS, LookFrame, STANDARD_ANIMATIONS,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnimationScheduler {
    spec: AnimationSpec,
    elapsed: Duration,
}

impl AnimationScheduler {
    pub fn new(state: AnimationState) -> Self {
        let spec = STANDARD_ANIMATIONS
            .into_iter()
            .find(|spec| spec.state() == state)
            .expect("every animation state must have a standard specification");
        Self {
            spec,
            elapsed: Duration::ZERO,
        }
    }

    pub const fn state(self) -> AnimationState {
        self.spec.state()
    }

    pub const fn elapsed(self) -> Duration {
        self.elapsed
    }

    pub fn loop_duration(self) -> Duration {
        self.spec.frames().map(FrameDescriptor::duration).sum()
    }

    pub fn current_frame(self) -> FrameDescriptor {
        let mut frame_end = Duration::ZERO;
        for frame in self.spec.frames() {
            frame_end += frame.duration();
            if self.elapsed < frame_end {
                return frame;
            }
        }
        unreachable!("scheduler elapsed time must remain inside its animation loop")
    }

    pub fn advance(&mut self, delta: Duration) -> FrameDescriptor {
        let loop_nanos = self.loop_duration().as_nanos();
        let elapsed_nanos = (self.elapsed.as_nanos() + delta.as_nanos()) % loop_nanos;
        self.elapsed = Duration::from_nanos(
            elapsed_nanos
                .try_into()
                .expect("standard animation loops must fit in u64 nanoseconds"),
        );
        self.current_frame()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LookDirectionSelector {
    deadzone: f64,
}

impl LookDirectionSelector {
    pub fn new(deadzone: f64) -> Result<Self, DirectionLookupError> {
        if !deadzone.is_finite() || deadzone < 0.0 {
            return Err(DirectionLookupError::InvalidDeadzone);
        }
        Ok(Self { deadzone })
    }

    pub const fn deadzone(self) -> f64 {
        self.deadzone
    }

    pub fn select(self, x: f64, y: f64) -> Option<LookFrame> {
        if !x.is_finite() || !y.is_finite() || x.hypot(y) <= self.deadzone {
            return None;
        }

        let clockwise_from_up = x.atan2(-y).to_degrees().rem_euclid(360.0);
        let index = ((clockwise_from_up / 22.5).round() as usize) % LOOK_DIRECTIONS.len();
        Some(LOOK_DIRECTIONS[index])
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum DirectionLookupError {
    #[error("look direction deadzone must be finite and non-negative")]
    InvalidDeadzone,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vector_at(degrees: f64, magnitude: f64) -> (f64, f64) {
        let radians = degrees.to_radians();
        (radians.sin() * magnitude, -radians.cos() * magnitude)
    }

    #[test]
    fn every_standard_row_uses_exact_frame_boundaries_and_wraps() {
        for spec in STANDARD_ANIMATIONS {
            let mut scheduler = AnimationScheduler::new(spec.state());
            let frames = spec.frames().collect::<Vec<_>>();
            assert_eq!(scheduler.current_frame(), frames[0]);

            let mut elapsed = Duration::ZERO;
            for (index, frame) in frames.iter().enumerate().skip(1) {
                elapsed += frames[index - 1].duration();
                let mut at_boundary = AnimationScheduler::new(spec.state());
                assert_eq!(at_boundary.advance(elapsed), *frame);
            }

            let loop_duration = scheduler.loop_duration();
            assert_eq!(scheduler.advance(loop_duration), frames[0]);
            assert_eq!(scheduler.elapsed(), Duration::ZERO);
            assert_eq!(
                scheduler.advance(loop_duration * 10 + frames[0].duration()),
                frames[1]
            );
        }
    }

    #[test]
    fn final_frame_remains_active_until_the_loop_boundary() {
        let mut scheduler = AnimationScheduler::new(AnimationState::Idle);
        let loop_duration = scheduler.loop_duration();
        assert_eq!(
            scheduler.advance(loop_duration - Duration::from_nanos(1)),
            STANDARD_ANIMATIONS[0].frames().last().unwrap()
        );
        assert_eq!(
            scheduler.advance(Duration::from_nanos(1)),
            STANDARD_ANIMATIONS[0].frames().next().unwrap()
        );
    }

    #[test]
    fn direction_centers_map_clockwise_in_screen_coordinates() {
        let selector = LookDirectionSelector::new(0.0).unwrap();
        for expected in LOOK_DIRECTIONS {
            let (x, y) = vector_at(expected.degrees().into(), 10.0);
            assert_eq!(selector.select(x, y), Some(expected));
        }
    }

    #[test]
    fn nearest_direction_rounds_across_zero_and_half_step_boundaries() {
        let selector = LookDirectionSelector::new(0.0).unwrap();
        let direction = |degrees| {
            let (x, y) = vector_at(degrees, 1.0);
            selector.select(x, y).unwrap().index()
        };

        assert_eq!(direction(11.249), 0);
        assert_eq!(direction(11.25), 1);
        assert_eq!(direction(11.251), 1);
        assert_eq!(direction(348.749), 15);
        assert_eq!(direction(348.75), 0);
        assert_eq!(direction(348.751), 0);
    }

    #[test]
    fn deadzone_is_inclusive_and_invalid_vectors_fail_closed() {
        let selector = LookDirectionSelector::new(5.0).unwrap();
        assert_eq!(selector.select(0.0, 0.0), None);
        assert_eq!(selector.select(3.0, 4.0), None);
        assert_eq!(selector.select(3.000_001, 4.0).unwrap().index(), 6);
        assert_eq!(selector.select(f64::NAN, 1.0), None);
        assert_eq!(selector.select(1.0, f64::INFINITY), None);
        assert_eq!(
            LookDirectionSelector::new(-1.0),
            Err(DirectionLookupError::InvalidDeadzone)
        );
        assert_eq!(
            LookDirectionSelector::new(f64::INFINITY),
            Err(DirectionLookupError::InvalidDeadzone)
        );
    }
}
