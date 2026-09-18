use std::time::Duration;

use leptos::prelude::*;
use lili_core::AppearanceView;
use lili_pet::{
    ATLAS_COLUMNS, ATLAS_ROWS, AnimationScheduler, AnimationState, LOOK_DIRECTIONS, LookFrame,
    NEUTRAL_LOOK_CELL, STANDARD_ANIMATIONS,
};

use super::{AppearancePreviewController, AtlasFrame, animation_token, frame_transform};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SpriteTarget {
    Animation(AnimationState),
    Look(LookFrame),
    Neutral,
    Cell(AtlasFrame),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SpritePlayback {
    pub target: SpriteTarget,
    pub playing: bool,
    scheduler: Option<AnimationScheduler>,
}

impl SpritePlayback {
    pub fn new(target: SpriteTarget) -> Self {
        let scheduler = match target {
            SpriteTarget::Animation(state) => Some(AnimationScheduler::new(state)),
            _ => None,
        };
        Self {
            target,
            playing: scheduler.is_some(),
            scheduler,
        }
    }

    pub fn frame(self) -> AtlasFrame {
        match self.target {
            SpriteTarget::Animation(_) => self
                .scheduler
                .expect("animation has a scheduler")
                .current_frame()
                .into(),
            SpriteTarget::Look(frame) => frame.into(),
            SpriteTarget::Neutral => AtlasFrame {
                row: NEUTRAL_LOOK_CELL.row(),
                column: NEUTRAL_LOOK_CELL.column(),
            },
            SpriteTarget::Cell(frame) => frame,
        }
    }

    #[cfg(any(test, feature = "hydrate"))]
    pub fn tick(&mut self, delta: Duration, reduced_motion: bool) -> AtlasFrame {
        if self.playing
            && !reduced_motion
            && let Some(scheduler) = &mut self.scheduler
        {
            scheduler.advance(delta);
        }
        self.frame()
    }

    fn seek(&mut self, column: usize) {
        let SpriteTarget::Animation(state) = self.target else {
            return;
        };
        let spec = STANDARD_ANIMATIONS
            .into_iter()
            .find(|spec| spec.state() == state)
            .unwrap();
        let elapsed: Duration = spec
            .frames()
            .take(column % spec.frame_count())
            .map(|frame| frame.duration())
            .sum();
        let mut scheduler = AnimationScheduler::new(state);
        scheduler.advance(elapsed);
        self.scheduler = Some(scheduler);
        self.playing = false;
    }

    fn step(&mut self, forward: bool) {
        let SpriteTarget::Animation(state) = self.target else {
            return;
        };
        let count = STANDARD_ANIMATIONS
            .into_iter()
            .find(|spec| spec.state() == state)
            .unwrap()
            .frame_count();
        let column = usize::from(self.frame().column);
        self.seek((column + if forward { 1 } else { count - 1 }) % count);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum SpriteSection {
    Animations,
    Directions,
    Sheet,
}

fn animation_label(state: AnimationState) -> &'static str {
    match state {
        AnimationState::Idle => "Idle",
        AnimationState::RunningRight => "Run right",
        AnimationState::RunningLeft => "Run left",
        AnimationState::Waving => "Wave",
        AnimationState::Jumping => "Jump",
        AnimationState::Failed => "Failed",
        AnimationState::Waiting => "Waiting",
        AnimationState::Running => "Running",
        AnimationState::Review => "Review",
    }
}

fn cell_label(frame: AtlasFrame) -> String {
    if frame.row == NEUTRAL_LOOK_CELL.row() && frame.column == NEUTRAL_LOOK_CELL.column() {
        return "Neutral pose".to_owned();
    }
    if let Some(look) = LOOK_DIRECTIONS
        .into_iter()
        .find(|look| look.row() == frame.row && look.column() == frame.column)
    {
        return format!("Look {}°", look.degrees());
    }
    if let Some(spec) = STANDARD_ANIMATIONS
        .into_iter()
        .find(|spec| spec.row() == frame.row && usize::from(frame.column) < spec.frame_count())
    {
        return format!(
            "{} · frame {}",
            animation_label(spec.state()),
            frame.column + 1
        );
    }
    "Unused by runtime".to_owned()
}

fn update_sprite(
    preview: RwSignal<AppearancePreviewController>,
    preview_frame: RwSignal<AtlasFrame>,
    update: impl FnOnce(&mut SpritePlayback),
) {
    preview.update(|preview| {
        if let Some(sprite) = &mut preview.sprite {
            update(sprite);
        }
    });
    preview_frame.set(preview.get_untracked().frame());
}

fn select_sprite(
    preview: RwSignal<AppearancePreviewController>,
    preview_frame: RwSignal<AtlasFrame>,
    target: SpriteTarget,
) {
    preview.update(|preview| preview.select_sprite(target));
    preview_frame.set(preview.get_untracked().frame());
}

#[component]
pub(super) fn SpriteBrowser(
    appearance: RwSignal<AppearanceView>,
    preview: RwSignal<AppearancePreviewController>,
    preview_frame: RwSignal<AtlasFrame>,
    reduced_motion: RwSignal<bool>,
) -> impl IntoView {
    let section = RwSignal::new(SpriteSection::Animations);
    let target = Memo::new(move |_| preview.get().sprite.map(|sprite| sprite.target));
    let asset_url = Signal::derive(move || {
        let appearance = appearance.get();
        appearance
            .pets
            .iter()
            .find(|pet| Some(&pet.id) == appearance.selected_pet_id.as_ref())
            .map(|pet| format!("/pet-assets/{}", pet.asset_id))
            .unwrap_or_default()
    });
    let sections = [
        (
            SpriteSection::Animations,
            "Animations",
            SpriteTarget::Animation(AnimationState::Idle),
        ),
        (
            SpriteSection::Directions,
            "Look directions",
            SpriteTarget::Neutral,
        ),
        (
            SpriteSection::Sheet,
            "Sprite sheet",
            SpriteTarget::Cell(AtlasFrame { row: 0, column: 0 }),
        ),
    ]
    .into_iter()
    .map(|(value, label, initial)| {
        view! {
            <button class="appearance-sprite-button" type="button"
                aria-pressed=move || (section.get() == value).to_string()
                on:click=move |_| {
                    if section.get_untracked() != value {
                        section.set(value);
                        select_sprite(preview, preview_frame, initial);
                    }
                }
            >{label}</button>
        }
    })
    .collect_view();

    view! {
        <div class="appearance-sprite-browser">
            <div class="appearance-sprite-toolbar" role="group" aria-label="Sprite category">{sections}</div>
            <Show when=move || section.get() == SpriteSection::Animations>
                <div class="appearance-sprite-toolbar" role="group" aria-label="Animations">
                    {STANDARD_ANIMATIONS.into_iter().map(|spec| {
                        let state = spec.state();
                        view! {
                            <button class="appearance-sprite-button" type="button"
                                data-animation=animation_token(state)
                                aria-pressed=move || (target.get() == Some(SpriteTarget::Animation(state))).to_string()
                                on:click=move |_| select_sprite(preview, preview_frame, SpriteTarget::Animation(state))
                            >{animation_label(state)}</button>
                        }
                    }).collect_view()}
                </div>
            </Show>
            <div class="appearance-sprite-stage">
                <div class="appearance-sprite-image" role="img" aria-label=move || cell_label(preview_frame.get())>
                    <img class="appearance-pet-atlas" src=move || asset_url.get() alt="" draggable="false"
                        data-frame-row=move || preview_frame.get().row
                        data-frame-column=move || preview_frame.get().column
                        style:transform=move || frame_transform(preview_frame.get())/>
                </div>
                <span class="appearance-sprite-caption">{move || cell_label(preview_frame.get())}</span>
            </div>
            <Show when=move || section.get() == SpriteSection::Animations>
                <div class="appearance-sprite-toolbar" role="group" aria-label="Playback controls">
                    <button class="appearance-sprite-button" type="button" aria-label="Previous frame"
                        on:click=move |_| update_sprite(preview, preview_frame, |sprite| sprite.step(false))>"←"</button>
                    <button class="appearance-sprite-button" type="button" disabled=move || reduced_motion.get()
                        on:click=move |_| update_sprite(preview, preview_frame, |sprite| sprite.playing = !sprite.playing)>
                        {move || if !reduced_motion.get() && preview.get().sprite.is_some_and(|sprite| sprite.playing) { "Pause" } else { "Play" }}
                    </button>
                    <button class="appearance-sprite-button" type="button" aria-label="Next frame"
                        on:click=move |_| update_sprite(preview, preview_frame, |sprite| sprite.step(true))>"→"</button>
                    <span class="appearance-sprite-caption">{move || {
                        let count = match target.get() {
                            Some(SpriteTarget::Animation(state)) => STANDARD_ANIMATIONS.into_iter().find(|spec| spec.state() == state).unwrap().frame_count(),
                            _ => 1,
                        };
                        format!("Frame {} / {}", preview_frame.get().column + 1, count)
                    }}</span>
                </div>
                <Show when=move || reduced_motion.get()>
                    <p class="appearance-sprite-hint">"Reduced motion is on. Select or step through frames manually."</p>
                </Show>
                <div class="appearance-sprite-grid appearance-animation-frames" role="group" aria-label="Animation frames">
                    {move || match target.get() {
                        Some(SpriteTarget::Animation(state)) => STANDARD_ANIMATIONS.into_iter().find(|spec| spec.state() == state).unwrap().frames().map(|frame| {
                            let frame = AtlasFrame::from(frame);
                            view! {
                                <SpriteThumbnail asset_url frame label=cell_label(frame)
                                    caption=format!("{}", frame.column + 1)
                                    selected=Signal::derive(move || preview_frame.get() == frame)
                                    on_select=move || update_sprite(preview, preview_frame, |sprite| sprite.seek(usize::from(frame.column)))/>
                            }
                        }).collect_view(),
                        _ => Vec::new().collect_view(),
                    }}
                </div>
            </Show>
            <Show when=move || section.get() == SpriteSection::Directions>
                <p class="appearance-sprite-hint">"Choose a fixed direction. Angles run clockwise from up."</p>
                <div class="appearance-sprite-grid" role="group" aria-label="Look directions">
                    {std::iter::once(SpriteTarget::Neutral).chain(LOOK_DIRECTIONS.into_iter().map(SpriteTarget::Look)).map(|value| {
                        let frame = SpritePlayback::new(value).frame();
                        view! {
                            <SpriteThumbnail asset_url frame label=cell_label(frame)
                                caption=match value {
                                    SpriteTarget::Look(look) => format!("{}°", look.degrees()),
                                    _ => "Neutral".to_owned(),
                                }
                                selected=Signal::derive(move || target.get() == Some(value))
                                on_select=move || select_sprite(preview, preview_frame, value)/>
                        }
                    }).collect_view()}
                </div>
            </Show>
            <Show when=move || section.get() == SpriteSection::Sheet>
                <p class="appearance-sprite-hint">"All 88 cells, including cells unused by the runtime. Select a cell to inspect it."</p>
                <div class="appearance-sheet-scroll" tabindex="0" role="region" aria-label="Complete sprite sheet">
                    <div class="appearance-sprite-grid appearance-sheet-grid">
                        {(0..ATLAS_ROWS).flat_map(|row| (0..ATLAS_COLUMNS).map(move |column| AtlasFrame { row, column })).map(|frame| {
                            let label = format!("Row {}, column {} · {}", frame.row + 1, frame.column + 1, cell_label(frame));
                            view! {
                                <SpriteThumbnail asset_url frame label
                                    selected=Signal::derive(move || target.get() == Some(SpriteTarget::Cell(frame)))
                                    on_select=move || select_sprite(preview, preview_frame, SpriteTarget::Cell(frame))/>
                            }
                        }).collect_view()}
                    </div>
                </div>
            </Show>
        </div>
    }
}

#[component]
fn SpriteThumbnail(
    asset_url: Signal<String>,
    frame: AtlasFrame,
    label: String,
    #[prop(optional)] caption: Option<String>,
    selected: Signal<bool>,
    on_select: impl Fn() + Send + Sync + 'static,
) -> impl IntoView {
    let unused = cell_label(frame) == "Unused by runtime";
    let caption = caption.unwrap_or_else(|| {
        if unused {
            "Unused".to_owned()
        } else {
            format!("{}·{}", frame.row + 1, frame.column + 1)
        }
    });
    view! {
        <button class="appearance-sprite-thumbnail" class:appearance-sprite-unused=unused
            type="button" aria-label=label.clone() title=label
            aria-pressed=move || selected.get().to_string()
            data-row=frame.row data-column=frame.column
            on:focus=move |event| {
                #[cfg(feature = "hydrate")]
                {
                    use wasm_bindgen::JsCast;
                    if let Some(element) = event.current_target().and_then(|target| target.dyn_into::<web_sys::Element>().ok()) {
                        // Native focus may leave a partially visible cell clipped by a nested scroller.
                        let options = web_sys::ScrollIntoViewOptions::new();
                        options.set_block(web_sys::ScrollLogicalPosition::Nearest);
                        options.set_inline(web_sys::ScrollLogicalPosition::Nearest);
                        element.scroll_into_view_with_scroll_into_view_options(&options);
                    }
                }
                #[cfg(not(feature = "hydrate"))]
                let _ = event;
            }
            on:click=move |_| on_select()>
            <span class="appearance-sprite-thumb-image">
                <img class="appearance-pet-atlas" src=move || asset_url.get() alt="" draggable="false"
                    style:transform=frame_transform(frame)/>
            </span>
            <span class="appearance-sprite-cell-label">{caption}</span>
        </button>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_animation_can_seek_step_wrap_and_resume_at_shared_frame_boundaries() {
        for spec in STANDARD_ANIMATIONS {
            let mut playback = SpritePlayback::new(SpriteTarget::Animation(spec.state()));
            let first = playback.frame();
            assert!(playback.playing);
            assert_eq!(playback.tick(Duration::from_secs(10), true), first);
            for frame in spec.frames() {
                playback.seek(usize::from(frame.column()));
                assert_eq!(playback.frame(), AtlasFrame::from(frame));
                assert_eq!(playback.tick(Duration::from_secs(10), false), frame.into());
                playback.playing = true;
                assert_eq!(
                    playback.tick(frame.duration(), false).column as usize,
                    (usize::from(frame.column()) + 1) % spec.frame_count()
                );
            }
            playback.seek(0);
            playback.step(false);
            assert_eq!(playback.frame().column as usize, spec.frame_count() - 1);
            playback.step(true);
            assert_eq!(playback.frame().column, 0);
            assert!(!playback.playing);
            assert_eq!(playback.tick(Duration::from_secs(10), true).column, 0);
        }
    }

    #[test]
    fn directions_neutral_and_every_sheet_cell_are_static_and_classified() {
        for target in std::iter::once(SpriteTarget::Neutral)
            .chain(LOOK_DIRECTIONS.into_iter().map(SpriteTarget::Look))
        {
            let mut playback = SpritePlayback::new(target);
            let frame = playback.frame();
            assert!(!playback.playing);
            assert_ne!(cell_label(frame), "Unused by runtime");
            assert_eq!(playback.tick(Duration::from_secs(10), false), frame);
        }
        let mut unused = 0;
        for row in 0..ATLAS_ROWS {
            for column in 0..ATLAS_COLUMNS {
                let frame = AtlasFrame { row, column };
                let mut playback = SpritePlayback::new(SpriteTarget::Cell(frame));
                assert_eq!(playback.tick(Duration::from_secs(10), false), frame);
                unused += usize::from(cell_label(frame) == "Unused by runtime");
            }
        }
        assert_eq!(unused, 14);
    }
}
