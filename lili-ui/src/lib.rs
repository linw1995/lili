use leptos::prelude::*;
use lili_core::{
    PetActionFeedbackKind, PetActionFeedbackPresentation, PetLifecycleState, PetNotificationKind,
    PetNotificationPresentation, PetPresentationState,
};
#[cfg(any(test, feature = "hydrate"))]
use lili_pet::LookDirectionSelector;
use lili_pet::{AnimationScheduler, AnimationState, FrameDescriptor, LookFrame};

#[cfg(any(test, feature = "hydrate"))]
const WAVE_DURATION_MS: u64 = 700;
#[cfg(any(test, feature = "hydrate"))]
const JUMP_DURATION_MS: u64 = 840;
#[cfg(any(test, feature = "hydrate"))]
const DOUBLE_CLICK_DELAY_MS: u64 = 250;
#[cfg(any(test, feature = "hydrate"))]
const DRAG_DISTANCE_PX: f64 = 5.0;
#[cfg(any(test, feature = "hydrate"))]
const DRAG_VELOCITY_PX_PER_MS: f64 = 0.12;
#[cfg(any(test, feature = "hydrate"))]
const DRAG_ANIMATION_HOLD_MS: u64 = 120;
#[cfg(feature = "hydrate")]
const PET_CENTER_X: f64 = 96.0;
#[cfg(feature = "hydrate")]
const PET_CENTER_Y: f64 = 104.0;

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
    let wall_clock = RwSignal::new(0_u64);
    let reduced_motion = RwSignal::new(presentation.get_untracked().reduced_motion);
    #[cfg(feature = "hydrate")]
    {
        reduced_motion
            .set(presentation.get_untracked().reduced_motion || system_prefers_reduced_motion());
        let pointer = RwSignal::new(PointerTracker::default());
        let clicks = RwSignal::new(ClickDisambiguator::default());
        let gaze = RwSignal::new(None::<LookFrame>);
        connect_presentation_stream(presentation, controller);
        start_animation_clock(
            controller,
            AnimationClockSignals {
                animation,
                frame,
                clicks,
                gaze,
                wall_clock,
                presentation,
                reduced_motion,
            },
        );
        let pet_view = view! {
            <section
                class="pet-sprite"
                role="button"
                tabindex="0"
                aria-keyshortcuts="Enter Space"
                aria-label=move || {
                    let state = presentation.get();
                    format!("{}, {}", state.pet_label, state.lifecycle.as_str())
                }
                data-hit-region="pet"
                on:pointerdown=move |event| {
                    if !event.is_primary() || event.button() != 0 {
                        return;
                    }
                    event.prevent_default();
                    capture_pointer(&event);
                    pointer.update(|pointer| pointer.press(
                        f64::from(event.screen_x()),
                        f64::from(event.screen_y()),
                        event.time_stamp().max(0.0) as u64,
                    ));
                    gaze.set(None);
                }
                on:pointermove=move |event| {
                    if !event.is_primary() {
                        return;
                    }
                    let motion_is_reduced = reduced_motion.get_untracked();
                    let (offset_x, offset_y) = pointer_offset(&event);
                    let update = pointer.write().move_to(
                        f64::from(event.screen_x()),
                        f64::from(event.screen_y()),
                        event.time_stamp().max(0.0) as u64,
                        offset_x - PET_CENTER_X,
                        offset_y - PET_CENTER_Y,
                    );
                    match update {
                        PointerUpdate::Gaze(next) => {
                            gaze.set((!motion_is_reduced).then_some(next).flatten());
                        }
                        PointerUpdate::DragVelocity(velocity) => {
                            gaze.set(None);
                            if let Some(delta) = velocity.window_delta {
                                move_native_window(delta.x, delta.y);
                                if !motion_is_reduced {
                                    controller.update(|controller| {
                                        controller.set_drag_velocity(
                                            velocity.x,
                                            animation_clock_ms(),
                                        );
                                    });
                                }
                            }
                        }
                    }
                }
                on:pointerup=move |event| {
                    if !event.is_primary() {
                        return;
                    }
                    release_pointer(&event);
                    let dragged = pointer.write().release();
                    controller.update(|controller| controller.end_drag(animation_clock_ms()));
                    gaze.set(None);
                    if dragged {
                        commit_native_window_position();
                    } else if clicks.write().release(event.time_stamp().max(0.0) as u64)
                        == ClickDecision::Double
                    {
                        controller.update(|controller| controller.trigger_jump(animation_clock_ms()));
                        activate_native_pet("pet_double_click");
                    }
                }
                on:pointercancel=move |event| {
                    if !event.is_primary() {
                        return;
                    }
                    release_pointer(&event);
                    let dragged = pointer.write().cancel();
                    controller.update(|controller| controller.end_drag(animation_clock_ms()));
                    gaze.set(None);
                    if dragged {
                        commit_native_window_position();
                    }
                }
                on:pointerleave=move |_| {
                    if !pointer.get_untracked().pressed() {
                        gaze.set(None);
                    }
                }
                on:keydown=move |event| {
                    if matches!(event.key().as_str(), "Enter" | " ") {
                        event.prevent_default();
                        controller.update(|controller| {
                            controller.trigger_wave(animation_clock_ms());
                        });
                        activate_native_pet("pet_click");
                    }
                }
            >
                <PetImage presentation frame/>
            </section>
        };
        view! {
            <PetShell presentation animation wall_clock reduced_motion pet_view/>
        }
    }
    #[cfg(not(feature = "hydrate"))]
    let pet_view = view! {
        <section
            class="pet-sprite"
            role="button"
            tabindex="0"
            aria-keyshortcuts="Enter Space"
            aria-label=move || {
                let state = presentation.get();
                format!("{}, {}", state.pet_label, state.lifecycle.as_str())
            }
            data-hit-region="pet"
        >
            <PetImage presentation frame/>
        </section>
    };
    #[cfg(not(feature = "hydrate"))]
    view! {
        <PetShell presentation animation wall_clock reduced_motion pet_view/>
    }
}

#[component]
fn PetShell(
    presentation: RwSignal<PetPresentationState>,
    animation: RwSignal<AnimationState>,
    wall_clock: RwSignal<u64>,
    reduced_motion: RwSignal<bool>,
    pet_view: impl IntoView + 'static,
) -> impl IntoView {
    #[cfg(feature = "hydrate")]
    let notification_cards = view! {
        <For
            each=move || presentation.get().notifications
            key=|notification| notification.activation_id.clone()
            children=move |notification| view! {
                <NotificationCard notification wall_clock/>
            }
        />
    };
    #[cfg(not(feature = "hydrate"))]
    let notification_cards = presentation
        .get_untracked()
        .notifications
        .into_iter()
        .map(|notification| view! { <NotificationCard notification wall_clock/> })
        .collect_view();
    #[cfg(feature = "hydrate")]
    let action_feedback = move || {
        presentation
            .get()
            .action_feedback
            .map(|feedback| view! { <ActionFeedback feedback/> })
    };
    #[cfg(not(feature = "hydrate"))]
    let action_feedback = presentation
        .get_untracked()
        .action_feedback
        .map(|feedback| view! { <ActionFeedback feedback/> });
    view! {
        <main
            id="lili-app"
            data-ssr-marker="lili-ready"
            data-presentation=move || serde_json::to_string(&presentation.get()).unwrap_or_default()
            data-revision=move || presentation.get().revision
            data-lifecycle=move || presentation.get().lifecycle.as_str()
            data-unread-count=move || presentation.get().unread_notification_count
            data-animation=move || animation_name(animation.get())
            data-reduced-motion=move || reduced_motion.get().to_string()
        >
            <aside
                class="notification-stack"
                aria-label="Session notifications"
                aria-live="polite"
                aria-relevant="additions removals"
            >
                {notification_cards}
            </aside>
            <aside
                class="action-feedback-region"
                aria-label="Action result"
                aria-live="polite"
                aria-atomic="true"
            >
                {action_feedback}
            </aside>
            {pet_view}
        </main>
    }
}

#[component]
fn ActionFeedback(feedback: PetActionFeedbackPresentation) -> impl IntoView {
    let role = if feedback.kind == PetActionFeedbackKind::Failure {
        "alert"
    } else {
        "status"
    };
    let action_id = feedback.action_id;
    let action_id_attribute = action_id.clone();
    view! {
        <div
            class="action-feedback"
            class:action-feedback-success=feedback.kind == PetActionFeedbackKind::Success
            class:action-feedback-failure=feedback.kind == PetActionFeedbackKind::Failure
            class:action-feedback-busy=feedback.kind == PetActionFeedbackKind::Busy
            role=role
            data-action-id=action_id_attribute
            data-action-result=feedback.kind.as_str()
        >
            <strong>{action_id}</strong>
            <span>{feedback.message}</span>
        </div>
    }
}

#[component]
fn NotificationCard(
    notification: PetNotificationPresentation,
    wall_clock: RwSignal<u64>,
) -> impl IntoView {
    let activation_id = notification.activation_id;
    let kind = notification.kind;
    let project_label = notification
        .project_label
        .unwrap_or_else(|| "Session".to_owned());
    let summary = notification.summary;
    let occurred_at_ms = notification.occurred_at_ms;
    let unread = notification.unread;
    let disclosure = match (
        notification.summary_redacted,
        notification.summary_truncated,
    ) {
        (true, true) => Some("Redacted and truncated"),
        (true, false) => Some("Redacted"),
        (false, true) => Some("Truncated"),
        (false, false) => None,
    };
    #[cfg(feature = "hydrate")]
    let controls = {
        let activate_id = activation_id.clone();
        let dismiss_id = activation_id.clone();
        view! {
            <button
                class="notification-activate"
                type="button"
                aria-label=format!("Open {} notification for {project_label}", notification_kind_label(kind))
                on:click=move |_| activate_native_notification(&activate_id)
            >
                "Open"
            </button>
            <button
                class="notification-dismiss"
                type="button"
                aria-label=format!("Dismiss {} notification for {project_label}", notification_kind_label(kind))
                on:click=move |_| dismiss_native_notification(&dismiss_id)
            >
                "Dismiss"
            </button>
        }
    };
    #[cfg(not(feature = "hydrate"))]
    let controls = view! {
        <button
            class="notification-activate"
            type="button"
            aria-label=format!("Open {} notification for {project_label}", notification_kind_label(kind))
        >
            "Open"
        </button>
        <button
            class="notification-dismiss"
            type="button"
            aria-label=format!("Dismiss {} notification for {project_label}", notification_kind_label(kind))
        >
            "Dismiss"
        </button>
    };
    view! {
        <article
            class="notification-card"
            class:notification-unread=unread
            data-notification-id=activation_id
            data-notification-kind=kind.as_str()
        >
            <div class="notification-heading">
                <span class="notification-kind">{notification_kind_label(kind)}</span>
                <time>{move || relative_time_label(occurred_at_ms, wall_clock.get())}</time>
            </div>
            <strong class="notification-project">{project_label}</strong>
            <p class="notification-summary">{summary}</p>
            {disclosure.map(|label| view! { <span class="notification-disclosure">{label}</span> })}
            <div class="notification-controls">{controls}</div>
        </article>
    }
}

const fn notification_kind_label(kind: PetNotificationKind) -> &'static str {
    match kind {
        PetNotificationKind::Attention => "Attention",
        PetNotificationKind::Completion => "Completed",
        PetNotificationKind::Failure => "Failed",
    }
}

fn relative_time_label(occurred_at_ms: u64, now_ms: u64) -> String {
    let elapsed_seconds = now_ms.saturating_sub(occurred_at_ms) / 1_000;
    match elapsed_seconds {
        0..60 => "Now".to_owned(),
        60..3_600 => format!("{}m ago", elapsed_seconds / 60),
        3_600..86_400 => format!("{}h ago", elapsed_seconds / 3_600),
        _ => format!("{}d ago", elapsed_seconds / 86_400),
    }
}

#[component]
fn PetImage(
    presentation: RwSignal<PetPresentationState>,
    frame: RwSignal<AtlasFrame>,
) -> impl IntoView {
    #[cfg(feature = "hydrate")]
    let image = view! {
        <img
            class="pet-atlas"
            src=move || presentation.get().pet_asset_id.map(|asset_id| format!("/pet-assets/{asset_id}"))
            alt=""
            aria-hidden="true"
            data-frame-row=move || frame.get().row
            data-frame-column=move || frame.get().column
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
                data-frame-row=frame.get_untracked().row
                data-frame-column=frame.get_untracked().column
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
    frame: AtlasFrame,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AtlasFrame {
    row: u8,
    column: u8,
}

impl From<FrameDescriptor> for AtlasFrame {
    fn from(frame: FrameDescriptor) -> Self {
        Self {
            row: frame.row(),
            column: frame.column(),
        }
    }
}

impl From<LookFrame> for AtlasFrame {
    fn from(frame: LookFrame) -> Self {
        Self {
            row: frame.row(),
            column: frame.column(),
        }
    }
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
    drag_animation: Option<AnimationState>,
    drag_animation_expires_at_ms: Option<u64>,
    selected: AnimationState,
    selected_at_ms: u64,
}

impl AnimationController {
    fn new(lifecycle: PetLifecycleState, now_ms: u64) -> Self {
        Self {
            lifecycle,
            temporary: None,
            drag_animation: None,
            drag_animation_expires_at_ms: None,
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
        self.render_with_gaze(now_ms, None)
    }

    fn render_with_gaze(&mut self, now_ms: u64, gaze: Option<LookFrame>) -> AnimationRender {
        self.render_mode(now_ms, gaze, false)
    }

    #[cfg(any(test, feature = "hydrate"))]
    fn render_reduced(&mut self, now_ms: u64) -> AnimationRender {
        self.render_mode(now_ms, None, true)
    }

    fn render_mode(
        &mut self,
        now_ms: u64,
        gaze: Option<LookFrame>,
        reduced_motion: bool,
    ) -> AnimationRender {
        if self
            .temporary
            .is_some_and(|temporary| now_ms >= temporary.expires_at_ms)
        {
            self.temporary = None;
        }
        if self
            .drag_animation_expires_at_ms
            .is_some_and(|expires_at_ms| now_ms >= expires_at_ms)
        {
            self.drag_animation = None;
            self.drag_animation_expires_at_ms = None;
        }
        self.refresh_selected(now_ms);
        if !reduced_motion
            && self.allows_gaze()
            && let Some(gaze) = gaze
        {
            return AnimationRender {
                animation: AnimationState::Idle,
                frame: gaze.into(),
            };
        }
        let elapsed = if reduced_motion {
            0
        } else {
            now_ms.saturating_sub(self.selected_at_ms)
        };
        let mut scheduler = AnimationScheduler::new(self.selected);
        let frame = scheduler.advance(std::time::Duration::from_millis(elapsed));
        AnimationRender {
            animation: self.selected,
            frame: frame.into(),
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

    #[cfg(any(test, feature = "hydrate"))]
    fn set_drag_velocity(&mut self, velocity_x: f64, now_ms: u64) {
        if matches!(
            self.lifecycle,
            PetLifecycleState::Waiting | PetLifecycleState::Failed
        ) {
            self.drag_animation = None;
            self.drag_animation_expires_at_ms = None;
        } else if velocity_x >= DRAG_VELOCITY_PX_PER_MS {
            self.drag_animation = Some(AnimationState::RunningRight);
            self.drag_animation_expires_at_ms = Some(now_ms.saturating_add(DRAG_ANIMATION_HOLD_MS));
        } else if velocity_x <= -DRAG_VELOCITY_PX_PER_MS {
            self.drag_animation = Some(AnimationState::RunningLeft);
            self.drag_animation_expires_at_ms = Some(now_ms.saturating_add(DRAG_ANIMATION_HOLD_MS));
        }
        self.refresh_selected(now_ms);
    }

    #[cfg(any(test, feature = "hydrate"))]
    fn end_drag(&mut self, now_ms: u64) {
        self.drag_animation = None;
        self.drag_animation_expires_at_ms = None;
        self.refresh_selected(now_ms);
    }

    fn allows_gaze(&self) -> bool {
        self.lifecycle == PetLifecycleState::Idle
            && self.temporary.is_none()
            && self.drag_animation.is_none()
    }

    fn refresh_selected(&mut self, now_ms: u64) {
        let selected = if matches!(
            self.lifecycle,
            PetLifecycleState::Waiting | PetLifecycleState::Failed
        ) {
            lifecycle_animation(self.lifecycle)
        } else {
            self.drag_animation
                .or_else(|| self.temporary.map(|temporary| temporary.animation))
                .unwrap_or_else(|| lifecycle_animation(self.lifecycle))
        };
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

fn frame_transform(frame: AtlasFrame) -> String {
    format!(
        "translate(-{}px,-{}px)",
        u32::from(frame.column) * lili_pet::CELL_WIDTH,
        u32::from(frame.row) * lili_pet::CELL_HEIGHT,
    )
}

fn animation_clock_ms() -> u64 {
    #[cfg(feature = "hydrate")]
    {
        web_sys::window()
            .and_then(|window| window.performance())
            .map_or(0, |performance| performance.now().max(0.0) as u64)
    }
    #[cfg(not(feature = "hydrate"))]
    0
}

#[cfg(feature = "hydrate")]
fn pointer_offset(event: &web_sys::PointerEvent) -> (f64, f64) {
    use wasm_bindgen::JsCast;

    event
        .current_target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        .map(|target| {
            let bounds = target.get_bounding_client_rect();
            (
                f64::from(event.client_x()) - bounds.left(),
                f64::from(event.client_y()) - bounds.top(),
            )
        })
        .unwrap_or_else(|| (f64::from(event.offset_x()), f64::from(event.offset_y())))
}

#[cfg(feature = "hydrate")]
fn capture_pointer(event: &web_sys::PointerEvent) {
    use wasm_bindgen::JsCast;

    if let Some(target) = event
        .current_target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
    {
        let _ = target.set_pointer_capture(event.pointer_id());
    }
}

#[cfg(feature = "hydrate")]
fn release_pointer(event: &web_sys::PointerEvent) {
    use wasm_bindgen::JsCast;

    if let Some(target) = event
        .current_target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
    {
        let _ = target.release_pointer_capture(event.pointer_id());
    }
}

#[cfg(feature = "hydrate")]
fn system_prefers_reduced_motion() -> bool {
    REDUCED_MOTION_QUERY.with(|query| query.as_ref().is_some_and(|query| query.matches()))
}

#[cfg(any(test, feature = "hydrate"))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct PointerTracker {
    pressed: Option<PointerSample>,
    last: Option<PointerSample>,
    dragging: bool,
}

#[cfg(any(test, feature = "hydrate"))]
#[derive(Clone, Copy, Debug, PartialEq)]
struct PointerSample {
    x: f64,
    y: f64,
    at_ms: u64,
}

#[cfg(any(test, feature = "hydrate"))]
#[derive(Clone, Copy, Debug, PartialEq)]
struct DragVelocity {
    x: f64,
    window_delta: Option<DragDelta>,
}

#[cfg(any(test, feature = "hydrate"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DragDelta {
    x: i32,
    y: i32,
}

#[cfg(any(test, feature = "hydrate"))]
#[derive(Clone, Copy, Debug, PartialEq)]
enum PointerUpdate {
    Gaze(Option<LookFrame>),
    DragVelocity(DragVelocity),
}

#[cfg(any(test, feature = "hydrate"))]
impl PointerTracker {
    fn press(&mut self, x: f64, y: f64, at_ms: u64) {
        let sample = PointerSample { x, y, at_ms };
        self.pressed = Some(sample);
        self.last = Some(sample);
        self.dragging = false;
    }

    fn move_to(&mut self, x: f64, y: f64, at_ms: u64, gaze_x: f64, gaze_y: f64) -> PointerUpdate {
        let Some(pressed) = self.pressed else {
            let gaze = LookDirectionSelector::new(18.0)
                .expect("fixed gaze deadzone is valid")
                .select(gaze_x, gaze_y);
            return PointerUpdate::Gaze(gaze);
        };
        let was_dragging = self.dragging;
        if (x - pressed.x).hypot(y - pressed.y) >= DRAG_DISTANCE_PX {
            self.dragging = true;
        }
        let previous = if self.dragging && !was_dragging {
            pressed
        } else {
            self.last.unwrap_or(pressed)
        };
        self.last = Some(PointerSample { x, y, at_ms });
        let elapsed_ms = at_ms.saturating_sub(previous.at_ms).max(1) as f64;
        let delta_x = x - previous.x;
        let delta_y = y - previous.y;
        PointerUpdate::DragVelocity(DragVelocity {
            x: delta_x / elapsed_ms,
            window_delta: self.dragging.then(|| DragDelta {
                x: delta_x.round() as i32,
                y: delta_y.round() as i32,
            }),
        })
    }

    fn release(&mut self) -> bool {
        let dragged = self.dragging;
        self.pressed = None;
        self.last = None;
        self.dragging = false;
        dragged
    }

    #[cfg(feature = "hydrate")]
    fn cancel(&mut self) -> bool {
        self.release()
    }

    fn pressed(&self) -> bool {
        self.pressed.is_some()
    }
}

#[cfg(any(test, feature = "hydrate"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ClickDisambiguator {
    pending_single_at_ms: Option<u64>,
}

#[cfg(any(test, feature = "hydrate"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClickDecision {
    Pending,
    Single,
    Double,
}

#[cfg(any(test, feature = "hydrate"))]
impl ClickDisambiguator {
    fn release(&mut self, at_ms: u64) -> ClickDecision {
        if self
            .pending_single_at_ms
            .is_some_and(|pending| at_ms.saturating_sub(pending) <= DOUBLE_CLICK_DELAY_MS)
        {
            self.pending_single_at_ms = None;
            ClickDecision::Double
        } else {
            self.pending_single_at_ms = Some(at_ms);
            ClickDecision::Pending
        }
    }

    fn poll(&mut self, now_ms: u64) -> ClickDecision {
        if self
            .pending_single_at_ms
            .is_some_and(|pending| now_ms.saturating_sub(pending) > DOUBLE_CLICK_DELAY_MS)
        {
            self.pending_single_at_ms = None;
            ClickDecision::Single
        } else {
            ClickDecision::Pending
        }
    }
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
#[derive(Clone, Copy)]
struct AnimationClockSignals {
    animation: RwSignal<AnimationState>,
    frame: RwSignal<AtlasFrame>,
    clicks: RwSignal<ClickDisambiguator>,
    gaze: RwSignal<Option<LookFrame>>,
    wall_clock: RwSignal<u64>,
    presentation: RwSignal<PetPresentationState>,
    reduced_motion: RwSignal<bool>,
}

#[cfg(feature = "hydrate")]
fn start_animation_clock(
    controller: RwSignal<AnimationController>,
    signals: AnimationClockSignals,
) {
    use wasm_bindgen::{JsCast, closure::Closure};

    let Some(window) = web_sys::window() else {
        return;
    };
    let callback = Closure::<dyn FnMut()>::new(move || {
        let now_ms = animation_clock_ms();
        signals.wall_clock.set(js_sys::Date::now().max(0.0) as u64);
        let motion_is_reduced =
            signals.presentation.get_untracked().reduced_motion || system_prefers_reduced_motion();
        signals.reduced_motion.set(motion_is_reduced);
        if signals.clicks.write().poll(now_ms) == ClickDecision::Single {
            controller.update(|controller| controller.trigger_wave(now_ms));
            activate_native_pet("pet_click");
        }
        let render = if motion_is_reduced {
            signals.gaze.set(None);
            controller.write().render_reduced(now_ms)
        } else {
            controller
                .write()
                .render_with_gaze(now_ms, signals.gaze.get_untracked())
        };
        signals.animation.set(render.animation);
        signals.frame.set(render.frame);
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
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function commitNativeWindowPosition() {
  const invoke = window.__TAURI_INTERNALS__?.invoke;
  if (invoke) windowMovePromise.then(() => invoke('commit_window_position')).catch(() => {});
}

let queuedWindowDeltaX = 0;
let queuedWindowDeltaY = 0;
let windowMoveActive = false;
let windowMovePromise = Promise.resolve();

export function moveNativeWindow(deltaX, deltaY) {
  const invoke = window.__TAURI_INTERNALS__?.invoke;
  if (!invoke) return;
  queuedWindowDeltaX += deltaX;
  queuedWindowDeltaY += deltaY;
  if (windowMoveActive) return;
  windowMoveActive = true;
  windowMovePromise = (async () => {
    while (queuedWindowDeltaX !== 0 || queuedWindowDeltaY !== 0) {
      const nextX = queuedWindowDeltaX;
      const nextY = queuedWindowDeltaY;
      queuedWindowDeltaX = 0;
      queuedWindowDeltaY = 0;
      await invoke('move_window_by', { deltaX: nextX, deltaY: nextY });
    }
  })().catch(() => {
    queuedWindowDeltaX = 0;
    queuedWindowDeltaY = 0;
  }).finally(() => {
    windowMoveActive = false;
  });
}
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = commitNativeWindowPosition)]
    fn commit_native_window_position();

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = moveNativeWindow)]
    fn move_native_window(delta_x: i32, delta_y: i32);
}

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function activateNativeNotification(id) {
  void fetch('/api/v1/interactions', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ trigger: 'notification_click', notification_id: id }),
  });
}

export function activateNativePet(trigger) {
  void fetch('/api/v1/interactions', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ trigger, notification_id: null }),
  });
}

export function dismissNativeNotification(id) {
  void fetch(`/api/v1/notifications/${encodeURIComponent(id)}/dismiss`, { method: 'POST' });
}
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = activateNativeNotification)]
    fn activate_native_notification(id: &str);

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = activateNativePet)]
    fn activate_native_pet(trigger: &str);

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = dismissNativeNotification)]
    fn dismiss_native_notification(id: &str);
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
    static REDUCED_MOTION_QUERY: Option<web_sys::MediaQueryList> = web_sys::window()
        .and_then(|window| window.match_media("(prefers-reduced-motion: reduce)").ok().flatten());
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
                unread_notification_count: 1,
                notifications: vec![PetNotificationPresentation {
                    activation_id: "notification-safe".to_owned(),
                    kind: PetNotificationKind::Completion,
                    project_label: Some("Workspace".to_owned()),
                    summary: "Finished safely".to_owned(),
                    summary_truncated: true,
                    summary_redacted: false,
                    occurred_at_ms: 10,
                    unread: true,
                }],
                action_feedback: Some(PetActionFeedbackPresentation {
                    action_id: "open-session".to_owned(),
                    kind: PetActionFeedbackKind::Failure,
                    message: "Action could not start".to_owned(),
                    occurred_at_ms: 11,
                }),
                reduced_motion: false,
            }/>
        }
        .to_html();
        assert!(html.contains("class=\"pet-sprite\""));
        assert!(html.contains("class=\"pet-atlas\""));
        assert!(html.contains("data-hit-region=\"pet\""));
        assert!(html.contains("data-revision=\"7\""));
        assert!(html.contains("data-lifecycle=\"waiting\""));
        assert!(html.contains("data-unread-count=\"1\""));
        assert!(html.contains("data-reduced-motion=\"false\""));
        assert!(html.contains("role=\"button\""));
        assert!(html.contains("tabindex=\"0\""));
        assert!(html.contains("aria-keyshortcuts=\"Enter Space\""));
        assert!(html.contains("aria-live=\"polite\""));
        assert!(html.contains("/pet-assets/opaque-id"));
        assert!(html.contains("data-notification-id=\"notification-safe\""));
        assert!(html.contains("Finished safely"));
        assert!(html.contains("Truncated"));
        assert!(html.contains("data-action-id=\"open-session\""));
        assert!(html.contains("data-action-result=\"failure\""));
        assert!(html.contains("Action could not start"));
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
    fn notification_time_labels_are_bounded_and_relative() {
        assert_eq!(relative_time_label(10_000, 10_000), "Now");
        assert_eq!(relative_time_label(10_000, 130_000), "2m ago");
        assert_eq!(relative_time_label(10_000, 7_210_000), "2h ago");
        assert_eq!(relative_time_label(10_000, 172_810_000), "2d ago");
        assert_eq!(relative_time_label(20_000, 10_000), "Now");
    }

    #[test]
    fn reduced_motion_uses_stable_representative_frames_without_gaze() {
        let selector = LookDirectionSelector::new(0.0).unwrap();
        let gaze = selector.select(100.0, 0.0);
        let mut controller = AnimationController::new(PetLifecycleState::Idle, 0);
        let first = controller.render_reduced(0);
        let later = controller.render_reduced(10_000);
        assert_eq!(first, later);
        assert_eq!(first.frame.row, 0);
        assert_eq!(first.frame.column, 0);
        assert_ne!(controller.render_with_gaze(10_000, gaze).frame, first.frame);

        controller.trigger_wave(20_000);
        let waving = controller.render_reduced(20_000);
        assert_eq!(waving.animation, AnimationState::Waving);
        assert_eq!(waving.frame.column, 0);
    }

    #[test]
    fn accessible_palette_meets_text_and_focus_contrast_thresholds() {
        assert!(contrast_ratio([255, 255, 255], [24, 28, 36]) >= 4.5);
        assert!(contrast_ratio([205, 215, 229], [24, 28, 36]) >= 4.5);
        assert!(contrast_ratio([120, 215, 255], [24, 28, 36]) >= 3.0);
        let css = include_str!("../../web/lili.css");
        assert!(css.contains(":focus-visible"));
        assert!(css.contains("prefers-reduced-motion: reduce"));
    }

    fn contrast_ratio(foreground: [u8; 3], background: [u8; 3]) -> f64 {
        let foreground = relative_luminance(foreground);
        let background = relative_luminance(background);
        let (lighter, darker) = if foreground > background {
            (foreground, background)
        } else {
            (background, foreground)
        };
        (lighter + 0.05) / (darker + 0.05)
    }

    fn relative_luminance(color: [u8; 3]) -> f64 {
        let [red, green, blue] = color.map(|channel| {
            let channel = f64::from(channel) / 255.0;
            if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        });
        0.2126 * red + 0.7152 * green + 0.0722 * blue
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
            assert_eq!(render.frame.row, row);
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

    #[test]
    fn click_disambiguation_emits_exactly_one_single_or_double() {
        let mut clicks = ClickDisambiguator::default();
        assert_eq!(clicks.release(100), ClickDecision::Pending);
        assert_eq!(clicks.poll(350), ClickDecision::Pending);
        assert_eq!(clicks.poll(351), ClickDecision::Single);
        assert_eq!(clicks.poll(600), ClickDecision::Pending);

        assert_eq!(clicks.release(1_000), ClickDecision::Pending);
        assert_eq!(clicks.release(1_200), ClickDecision::Double);
        assert_eq!(clicks.poll(2_000), ClickDecision::Pending);
    }

    #[test]
    fn pointer_gaze_and_drag_velocity_are_hit_region_local() {
        let mut pointer = PointerTracker::default();
        let PointerUpdate::Gaze(Some(look)) = pointer.move_to(0.0, 0.0, 0, 100.0, 0.0) else {
            panic!("pointer outside a press should select gaze");
        };
        assert_eq!(look.index(), 4);

        pointer.press(100.0, 100.0, 10);
        let PointerUpdate::DragVelocity(velocity) = pointer.move_to(101.0, 100.0, 20, 0.0, 0.0)
        else {
            panic!("pressed pointer should report drag velocity");
        };
        assert_eq!(velocity.x, 0.1);
        assert_eq!(velocity.window_delta, None);
        assert!(!pointer.dragging);
        let PointerUpdate::DragVelocity(velocity) = pointer.move_to(110.0, 100.0, 30, 0.0, 0.0)
        else {
            panic!("drag should continue reporting velocity");
        };
        assert_eq!(velocity.x, 0.5);
        assert_eq!(velocity.window_delta, Some(DragDelta { x: 10, y: 0 }));
        assert!(pointer.release());
        assert!(!pointer.pressed());
    }

    #[test]
    fn drag_direction_and_gaze_obey_animation_priority() {
        let selector = LookDirectionSelector::new(0.0).unwrap();
        let gaze = selector.select(100.0, 0.0);
        let mut controller = AnimationController::new(PetLifecycleState::Idle, 0);
        let gaze_render = controller.render_with_gaze(0, gaze);
        assert!(gaze_render.frame.row >= 9);

        controller.set_drag_velocity(0.2, 10);
        assert_eq!(
            controller.render_with_gaze(10, gaze).animation,
            AnimationState::RunningRight
        );
        controller.set_drag_velocity(-0.2, 20);
        assert_eq!(controller.render(20).animation, AnimationState::RunningLeft);
        controller.set_drag_velocity(0.0, 30);
        assert_eq!(
            controller.render(139).animation,
            AnimationState::RunningLeft
        );
        assert_eq!(controller.render(140).animation, AnimationState::Idle);
        controller.set_drag_velocity(-0.2, 150);
        controller.end_drag(160);
        assert_eq!(controller.render(160).animation, AnimationState::Idle);

        controller.set_lifecycle(PetLifecycleState::Waiting, 40);
        controller.set_drag_velocity(1.0, 50);
        let waiting = controller.render_with_gaze(50, gaze);
        assert_eq!(waiting.animation, AnimationState::Waiting);
        assert_eq!(waiting.frame.row, 6);
    }
}
