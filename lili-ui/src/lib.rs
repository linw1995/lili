use leptos::prelude::*;
use lili_core::{PetLifecycleState, PetPresentationState};
use lili_pet::{AnimationScheduler, AnimationState, FrameDescriptor};

#[cfg(any(test, feature = "hydrate"))]
const WAVE_DURATION_MS: u64 = 700;
#[cfg(any(test, feature = "hydrate"))]
const JUMP_DURATION_MS: u64 = 840;

#[component]
pub fn App(presentation: PetPresentationState) -> impl IntoView {
    let presentation = RwSignal::new(presentation);
    let now_ms = animation_clock_ms();
    let controller = RwSignal::new(AnimationController::new(
        presentation.get_untracked().lifecycle,
        now_ms,
    ));
    let initial_render = controller.get_untracked().render(now_ms);
    let animation = RwSignal::new(initial_render.animation);
    let frame = RwSignal::new(initial_render.frame);
    #[cfg(feature = "hydrate")]
    {
        connect_presentation_stream(presentation, controller);
        start_animation_clock(controller, animation, frame);
    }
    #[cfg(feature = "hydrate")]
    let pet_view = view! {
        <section
            class="pet-sprite"
            aria-label=move || {
                let state = presentation.get();
                format!("{}, {}", state.pet_label, state.lifecycle.as_str())
            }
            data-tauri-drag-region="deep"
            on:click=move |_| controller.update(|controller| controller.trigger_wave(animation_clock_ms()))
            on:dblclick=move |_| controller.update(|controller| controller.trigger_jump(animation_clock_ms()))
        >
            <PetImage presentation frame/>
        </section>
    };
    #[cfg(not(feature = "hydrate"))]
    let pet_view = view! {
        <section
            class="pet-sprite"
            aria-label=move || {
                let state = presentation.get();
                format!("{}, {}", state.pet_label, state.lifecycle.as_str())
            }
            data-tauri-drag-region="deep"
        >
            <PetImage presentation frame/>
        </section>
    };
    view! {
        <main
            id="lili-app"
            data-ssr-marker="lili-ready"
            data-presentation=move || serde_json::to_string(&presentation.get()).unwrap_or_default()
            data-revision=move || presentation.get().revision
            data-lifecycle=move || presentation.get().lifecycle.as_str()
            data-unread-count=move || presentation.get().unread_notification_count
            data-animation=move || animation_name(animation.get())
        >
            {pet_view}
        </main>
    }
}

#[component]
fn PetImage(
    presentation: RwSignal<PetPresentationState>,
    frame: RwSignal<FrameDescriptor>,
) -> impl IntoView {
    #[cfg(feature = "hydrate")]
    let image = view! {
        <img
            class="pet-atlas"
            src=move || presentation.get().pet_asset_id.map(|asset_id| format!("/pet-assets/{asset_id}"))
            alt=""
            aria-hidden="true"
            style:animation="none"
            style:transform=move || frame_transform(frame.get())
        />
    };
    #[cfg(not(feature = "hydrate"))]
    let image = {
        let asset_source = presentation
            .get_untracked()
            .pet_asset_id
            .map(|asset_id| format!("/pet-assets/{asset_id}"));
        let style = format!(
            "animation:none;transform:{}",
            frame_transform(frame.get_untracked())
        );
        view! {
            <img
                class="pet-atlas"
                src=asset_source
                alt=""
                aria-hidden="true"
                style=style
            />
        }
    };
    view! {
        {image}
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AnimationRender {
    animation: AnimationState,
    frame: FrameDescriptor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TemporaryAnimation {
    animation: AnimationState,
    expires_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AnimationController {
    lifecycle: PetLifecycleState,
    temporary: Option<TemporaryAnimation>,
    selected: AnimationState,
    selected_at_ms: u64,
}

impl AnimationController {
    fn new(lifecycle: PetLifecycleState, now_ms: u64) -> Self {
        Self {
            lifecycle,
            temporary: None,
            selected: lifecycle_animation(lifecycle),
            selected_at_ms: now_ms,
        }
    }

    #[cfg(any(test, feature = "hydrate"))]
    fn set_lifecycle(&mut self, lifecycle: PetLifecycleState, now_ms: u64) {
        self.lifecycle = lifecycle;
        if matches!(
            lifecycle,
            PetLifecycleState::Waiting | PetLifecycleState::Failed
        ) {
            self.temporary = None;
        }
        self.refresh_selected(now_ms);
    }

    #[cfg(any(test, feature = "hydrate"))]
    fn trigger_wave(&mut self, now_ms: u64) {
        self.trigger(AnimationState::Waving, WAVE_DURATION_MS, now_ms);
    }

    #[cfg(any(test, feature = "hydrate"))]
    fn trigger_jump(&mut self, now_ms: u64) {
        self.trigger(AnimationState::Jumping, JUMP_DURATION_MS, now_ms);
    }

    fn render(&mut self, now_ms: u64) -> AnimationRender {
        if self
            .temporary
            .is_some_and(|temporary| now_ms >= temporary.expires_at_ms)
        {
            self.temporary = None;
        }
        self.refresh_selected(now_ms);
        let elapsed = now_ms.saturating_sub(self.selected_at_ms);
        let mut scheduler = AnimationScheduler::new(self.selected);
        let frame = scheduler.advance(std::time::Duration::from_millis(elapsed));
        AnimationRender {
            animation: self.selected,
            frame,
        }
    }

    #[cfg(any(test, feature = "hydrate"))]
    fn trigger(&mut self, animation: AnimationState, duration_ms: u64, now_ms: u64) {
        if matches!(
            self.lifecycle,
            PetLifecycleState::Waiting | PetLifecycleState::Failed
        ) {
            return;
        }
        self.temporary = Some(TemporaryAnimation {
            animation,
            expires_at_ms: now_ms.saturating_add(duration_ms),
        });
        self.refresh_selected(now_ms);
    }

    fn refresh_selected(&mut self, now_ms: u64) {
        let selected = self.temporary.map_or_else(
            || lifecycle_animation(self.lifecycle),
            |temporary| temporary.animation,
        );
        if selected != self.selected {
            self.selected = selected;
            self.selected_at_ms = now_ms;
        }
    }
}

fn lifecycle_animation(lifecycle: PetLifecycleState) -> AnimationState {
    match lifecycle {
        PetLifecycleState::Idle => AnimationState::Idle,
        PetLifecycleState::Running => AnimationState::Running,
        PetLifecycleState::Review => AnimationState::Review,
        PetLifecycleState::Failed => AnimationState::Failed,
        PetLifecycleState::Waiting => AnimationState::Waiting,
    }
}

fn animation_name(animation: AnimationState) -> &'static str {
    match animation {
        AnimationState::Idle => "idle",
        AnimationState::RunningRight => "running-right",
        AnimationState::RunningLeft => "running-left",
        AnimationState::Waving => "waving",
        AnimationState::Jumping => "jumping",
        AnimationState::Failed => "failed",
        AnimationState::Waiting => "waiting",
        AnimationState::Running => "running",
        AnimationState::Review => "review",
    }
}

fn frame_transform(frame: FrameDescriptor) -> String {
    format!(
        "translate(-{}px,-{}px)",
        u32::from(frame.column()) * lili_pet::CELL_WIDTH,
        u32::from(frame.row()) * lili_pet::CELL_HEIGHT,
    )
}

fn animation_clock_ms() -> u64 {
    #[cfg(feature = "hydrate")]
    {
        return web_sys::window()
            .and_then(|window| window.performance())
            .map_or(0, |performance| performance.now().max(0.0) as u64);
    }
    #[cfg(not(feature = "hydrate"))]
    0
}

#[cfg(any(test, feature = "hydrate"))]
#[derive(Clone, Debug, Eq, PartialEq)]
struct PresentationCursor {
    current: PetPresentationState,
}

#[cfg(any(test, feature = "hydrate"))]
impl PresentationCursor {
    fn new(current: PetPresentationState) -> Self {
        Self { current }
    }

    fn accept(&mut self, next: PetPresentationState) -> bool {
        if next.revision <= self.current.revision {
            return false;
        }
        self.current = next;
        true
    }
}

#[cfg(feature = "hydrate")]
fn connect_presentation_stream(
    presentation: RwSignal<PetPresentationState>,
    controller: RwSignal<AnimationController>,
) {
    use wasm_bindgen::{JsCast, closure::Closure};
    use web_sys::{EventSource, MessageEvent};

    let Ok(source) = EventSource::new("/presentation-events") else {
        return;
    };
    let callback = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let Some(serialized) = event.data().as_string() else {
            return;
        };
        let Ok(next) = serde_json::from_str::<PetPresentationState>(&serialized) else {
            return;
        };
        let mut cursor = PresentationCursor::new(presentation.get_untracked());
        if cursor.accept(next) {
            controller.update(|controller| {
                controller.set_lifecycle(cursor.current.lifecycle, animation_clock_ms());
            });
            presentation.set(cursor.current);
        }
    });
    for event_name in ["snapshot", "presentation"] {
        let _ =
            source.add_event_listener_with_callback(event_name, callback.as_ref().unchecked_ref());
    }
    PRESENTATION_STREAM.with(|stream| {
        if let Some(previous) = stream.borrow_mut().replace(PresentationStream {
            source,
            _callback: callback,
        }) {
            previous.source.close();
        }
    });
}

#[cfg(feature = "hydrate")]
fn start_animation_clock(
    controller: RwSignal<AnimationController>,
    animation: RwSignal<AnimationState>,
    frame: RwSignal<FrameDescriptor>,
) {
    use wasm_bindgen::{JsCast, closure::Closure};

    let Some(window) = web_sys::window() else {
        return;
    };
    let callback = Closure::<dyn FnMut()>::new(move || {
        let render = controller.write().render(animation_clock_ms());
        animation.set(render.animation);
        frame.set(render.frame);
    });
    let Ok(interval_id) = window.set_interval_with_callback_and_timeout_and_arguments_0(
        callback.as_ref().unchecked_ref(),
        16,
    ) else {
        return;
    };
    ANIMATION_CLOCK.with(|clock| {
        if let Some(previous) = clock.borrow_mut().replace(AnimationClock {
            window,
            interval_id,
            _callback: callback,
        }) {
            previous
                .window
                .clear_interval_with_handle(previous.interval_id);
        }
    });
}

#[cfg(feature = "hydrate")]
struct PresentationStream {
    source: web_sys::EventSource,
    _callback: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::MessageEvent)>,
}

#[cfg(feature = "hydrate")]
struct AnimationClock {
    window: web_sys::Window,
    interval_id: i32,
    _callback: wasm_bindgen::closure::Closure<dyn FnMut()>,
}

#[cfg(feature = "hydrate")]
thread_local! {
    static PRESENTATION_STREAM: std::cell::RefCell<Option<PresentationStream>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(feature = "hydrate")]
thread_local! {
    static ANIMATION_CLOCK: std::cell::RefCell<Option<AnimationClock>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn hydrate() {
    let presentation = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("lili-app"))
        .and_then(|element| element.get_attribute("data-presentation"))
        .and_then(|serialized| serde_json::from_str::<PetPresentationState>(&serialized).ok())
        .unwrap_or_default();
    leptos::mount::hydrate_body(move || {
        view! { <App presentation=presentation.clone()/> }
    });
}

#[cfg(test)]
mod tests {
    use lili_core::PetLifecycleState;

    use super::*;

    #[cfg(feature = "ssr")]
    #[test]
    fn ssr_shell_renders_only_the_native_presentation_model() {
        let html = view! {
            <App presentation=PetPresentationState {
                revision: 7,
                lifecycle: PetLifecycleState::Waiting,
                pet_asset_id: Some("opaque-id".to_owned()),
                pet_label: "Lili".to_owned(),
                unread_notification_count: 2,
            }/>
        }
        .to_html();
        assert!(html.contains("class=\"pet-sprite\""));
        assert!(html.contains("class=\"pet-atlas\""));
        assert!(html.contains("data-tauri-drag-region=\"deep\""));
        assert!(html.contains("data-revision=\"7\""));
        assert!(html.contains("data-lifecycle=\"waiting\""));
        assert!(html.contains("data-unread-count=\"2\""));
        assert!(html.contains("/pet-assets/opaque-id"));
        assert!(!html.contains("sessionId"));
        assert!(!html.contains("eventId"));
    }

    #[test]
    fn reconnect_cursor_rejects_replayed_and_stale_revisions() {
        let mut cursor = PresentationCursor::new(PetPresentationState {
            revision: 5,
            unread_notification_count: 1,
            ..PetPresentationState::default()
        });
        assert!(!cursor.accept(PetPresentationState {
            revision: 5,
            unread_notification_count: 2,
            ..PetPresentationState::default()
        }));
        assert!(!cursor.accept(PetPresentationState {
            revision: 4,
            unread_notification_count: 3,
            ..PetPresentationState::default()
        }));
        assert!(cursor.accept(PetPresentationState {
            revision: 6,
            unread_notification_count: 2,
            ..PetPresentationState::default()
        }));
        assert_eq!(cursor.current.revision, 6);
        assert_eq!(cursor.current.unread_notification_count, 2);
    }

    #[test]
    fn lifecycle_rows_and_temporary_overlays_return_deterministically() {
        let cases = [
            (PetLifecycleState::Idle, AnimationState::Idle, 0),
            (PetLifecycleState::Running, AnimationState::Running, 7),
            (PetLifecycleState::Review, AnimationState::Review, 8),
            (PetLifecycleState::Failed, AnimationState::Failed, 5),
            (PetLifecycleState::Waiting, AnimationState::Waiting, 6),
        ];
        for (lifecycle, animation, row) in cases {
            let mut controller = AnimationController::new(lifecycle, 100);
            let render = controller.render(100);
            assert_eq!(render.animation, animation);
            assert_eq!(render.frame.row(), row);
        }

        let mut controller = AnimationController::new(PetLifecycleState::Running, 100);
        controller.trigger_wave(200);
        assert_eq!(controller.render(200).animation, AnimationState::Waving);
        controller.set_lifecycle(PetLifecycleState::Review, 300);
        assert_eq!(controller.render(899).animation, AnimationState::Waving);
        assert_eq!(controller.render(900).animation, AnimationState::Review);
    }

    #[test]
    fn attention_and_failure_interrupt_temporary_animation() {
        let mut controller = AnimationController::new(PetLifecycleState::Idle, 0);
        controller.trigger_jump(10);
        assert_eq!(controller.render(10).animation, AnimationState::Jumping);
        controller.set_lifecycle(PetLifecycleState::Waiting, 20);
        assert_eq!(controller.render(20).animation, AnimationState::Waiting);
        controller.trigger_wave(30);
        assert_eq!(controller.render(30).animation, AnimationState::Waiting);
        controller.set_lifecycle(PetLifecycleState::Failed, 40);
        assert_eq!(controller.render(40).animation, AnimationState::Failed);
    }
}
