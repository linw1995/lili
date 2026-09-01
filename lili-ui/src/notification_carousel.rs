use leptos::prelude::*;
use lili_core::{PetNotificationKind, PetNotificationPresentation, PetPresentationState};

#[cfg(feature = "hydrate")]
use super::{
    activate_native_notification, animation_clock_ms, dismiss_native_notification,
    resize_notification_window,
};
use super::{notification_kind_label, relative_time_label};

#[component]
pub(super) fn NotificationCarousel(
    presentation: RwSignal<PetPresentationState>,
    wall_clock: RwSignal<u64>,
    reduced_motion: RwSignal<bool>,
) -> impl IntoView {
    let initial_ids = notifications_by_display_order(presentation.get_untracked().notifications)
        .into_iter()
        .map(|notification| notification.activation_id)
        .collect();
    let carousel = NotificationCarouselController::new(initial_ids, reduced_motion);
    #[cfg(feature = "hydrate")]
    {
        let reconcile_carousel = carousel.clone();
        Effect::new(move |_| {
            let ids = notifications_by_display_order(presentation.get().notifications)
                .into_iter()
                .map(|notification| notification.activation_id)
                .collect();
            reconcile_carousel.reconcile(ids);
        });
        let motion_carousel = carousel.clone();
        Effect::new(move |_| {
            if reduced_motion.get() {
                motion_carousel
                    .state
                    .update(NotificationCarouselState::jump_to_pending);
            }
        });
    }
    let cards_carousel = carousel.clone();
    #[cfg(feature = "hydrate")]
    let notification_cards = view! {
        <For
            each=move || notifications_by_keyboard_order(presentation.get().notifications)
            key=|notification| notification.activation_id.clone()
            children=move |notification| view! {
                <NotificationCard notification wall_clock carousel=cards_carousel.clone()/>
            }
        />
    };
    #[cfg(not(feature = "hydrate"))]
    let notification_cards = notifications_by_keyboard_order(
        presentation.get_untracked().notifications,
    )
    .into_iter()
    .map(|notification| {
        view! { <NotificationCard notification wall_clock carousel=cards_carousel.clone()/> }
    })
    .collect_view();
    #[cfg(feature = "hydrate")]
    let visible_count_carousel = carousel.clone();
    #[cfg(feature = "hydrate")]
    let wheel_carousel = carousel.clone();
    #[cfg(feature = "hydrate")]
    let more_top_carousel = carousel.clone();
    #[cfg(feature = "hydrate")]
    let more_bottom_carousel = carousel.clone();
    #[cfg(feature = "hydrate")]
    let transition_carousel = carousel.clone();
    #[cfg(feature = "hydrate")]
    let notification_stack = view! {
        <aside
            class="notification-stack"
            aria-label="Session notifications"
            aria-live="polite"
            aria-relevant="additions removals"
            data-notification-visible-count=move || visible_count_carousel.visible_count().to_string()
            class:notification-stack-more-top=move || more_top_carousel.has_more_top()
            class:notification-stack-more-bottom=move || more_bottom_carousel.has_more_bottom()
            on:wheel=move |event| wheel_carousel.handle_wheel(event)
            on:transitionend=move |event| transition_carousel.handle_transition_end(event)
        >
            {notification_cards}
            <span class="notification-more notification-more-top" aria-hidden="true"></span>
            <span class="notification-more notification-more-bottom" aria-hidden="true"></span>
        </aside>
    };
    #[cfg(not(feature = "hydrate"))]
    let notification_stack = view! {
        <aside
            class="notification-stack"
            aria-label="Session notifications"
            aria-live="polite"
            aria-relevant="additions removals"
            data-notification-visible-count=carousel.visible_count().to_string()
            class:notification-stack-more-top=carousel.has_more_top()
            class:notification-stack-more-bottom=carousel.has_more_bottom()
        >
            {notification_cards}
            <span class="notification-more notification-more-top" aria-hidden="true"></span>
            <span class="notification-more notification-more-bottom" aria-hidden="true"></span>
        </aside>
    };
    notification_stack
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NotificationCardVisual {
    pub(super) role: NotificationCardRole,
    pub(super) current: bool,
    pub(super) foreground: bool,
}

impl NotificationCardVisual {
    const HIDDEN: Self = Self {
        role: NotificationCardRole::Hidden,
        current: false,
        foreground: false,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NotificationCardRole {
    Hidden,
    TopBehind,
    Top,
    Bottom,
    BottomBehind,
}

#[derive(Clone, Debug, PartialEq)]
struct NotificationCarouselState {
    ordered_ids: Vec<String>,
    window_start: usize,
    foreground_index: Option<usize>,
    pending_focus: Option<usize>,
    pending_moves: i32,
    wheel_accumulator: f64,
    last_wheel_at_ms: Option<u64>,
    wheel_lock_until_ms: u64,
    last_wheel_direction: i32,
    transition_until_ms: u64,
}

impl NotificationCarouselState {
    #[cfg(any(test, feature = "hydrate"))]
    const CARD_HEIGHT: u32 = 58;
    #[cfg(any(test, feature = "hydrate"))]
    const CARD_GAP: u32 = 8;
    #[cfg(any(test, feature = "hydrate"))]
    const STACK_PADDING: u32 = 12;
    #[cfg(any(test, feature = "hydrate"))]
    const WHEEL_TRIGGER_DISTANCE: f64 = 24.0;
    #[cfg(feature = "hydrate")]
    const WHEEL_SCALE: f64 = 0.75;
    #[cfg(any(test, feature = "hydrate"))]
    const WHEEL_DEBOUNCE_MS: u64 = 80;
    #[cfg(any(test, feature = "hydrate"))]
    const TRANSITION_DURATION_MS: u64 = 460;
    #[cfg(any(test, feature = "hydrate"))]
    const WHEEL_LINE_HEIGHT: f64 = 16.0;
    #[cfg(any(test, feature = "hydrate"))]
    const WHEEL_PAGE_HEIGHT: f64 = 148.0;

    fn new(ordered_ids: Vec<String>) -> Self {
        let window_start = ordered_ids.len().saturating_sub(2);
        Self {
            foreground_index: (!ordered_ids.is_empty()).then_some(window_start),
            ordered_ids,
            window_start,
            pending_focus: None,
            pending_moves: 0,
            wheel_accumulator: 0.0,
            last_wheel_at_ms: None,
            wheel_lock_until_ms: 0,
            last_wheel_direction: 0,
            transition_until_ms: 0,
        }
    }

    fn visible_count(&self) -> usize {
        self.ordered_ids.len().min(2)
    }

    #[cfg(any(test, feature = "hydrate"))]
    fn stack_height(&self) -> u32 {
        match self.visible_count() {
            0 => 24,
            1 => Self::STACK_PADDING * 2 + Self::CARD_HEIGHT,
            _ => Self::STACK_PADDING * 2 + Self::CARD_HEIGHT * 2 + Self::CARD_GAP,
        }
    }

    fn maximum_window_start(&self) -> usize {
        self.ordered_ids.len().saturating_sub(2)
    }

    fn clamp_window_start(&self, start: usize) -> usize {
        start.min(self.maximum_window_start())
    }

    fn role_for_index(&self, index: usize) -> NotificationCardRole {
        let start = self.clamp_window_start(self.window_start);
        if self.ordered_ids.len() == 1 && index == start {
            return NotificationCardRole::Bottom;
        }
        if index == start {
            NotificationCardRole::Top
        } else if index == start.saturating_add(1) && index < self.ordered_ids.len() {
            NotificationCardRole::Bottom
        } else if start > 0 && index == start - 1 {
            NotificationCardRole::TopBehind
        } else if index == start.saturating_add(2) {
            NotificationCardRole::BottomBehind
        } else {
            NotificationCardRole::Hidden
        }
    }

    fn visual_for(&self, id: &str) -> NotificationCardVisual {
        let Some(index) = self
            .ordered_ids
            .iter()
            .position(|candidate| candidate == id)
        else {
            return NotificationCardVisual::HIDDEN;
        };
        let role = self.role_for_index(index);
        NotificationCardVisual {
            role,
            current: matches!(
                role,
                NotificationCardRole::Top | NotificationCardRole::Bottom
            ),
            foreground: self.foreground_index == Some(index)
                && !matches!(role, NotificationCardRole::Hidden),
        }
    }

    fn has_more_top(&self) -> bool {
        self.window_start > 0
    }

    fn has_more_bottom(&self) -> bool {
        self.window_start.saturating_add(2) < self.ordered_ids.len()
    }

    #[cfg(any(test, feature = "hydrate"))]
    fn reconcile(&mut self, ordered_ids: Vec<String>) -> bool {
        if self.ordered_ids == ordered_ids {
            return false;
        }

        let previous_window_start = self.window_start.min(self.maximum_window_start());
        let previous_foreground_id = self
            .foreground_index
            .and_then(|index| self.ordered_ids.get(index))
            .cloned();
        let removed_index = single_removal_index(&self.ordered_ids, &ordered_ids);
        let preserved_window_start = removed_index.map(|removed_index| {
            previous_window_start
                .saturating_sub((removed_index <= previous_window_start) as usize)
                .min(ordered_ids.len().saturating_sub(2))
        });

        self.ordered_ids = ordered_ids;
        self.window_start = preserved_window_start.unwrap_or_else(|| self.maximum_window_start());
        self.foreground_index = previous_foreground_id
            .and_then(|foreground_id| {
                self.ordered_ids
                    .iter()
                    .position(|candidate| candidate == &foreground_id)
            })
            .filter(|index| {
                *index >= self.window_start
                    && *index < self.window_start.saturating_add(2)
                    && *index < self.ordered_ids.len()
            })
            .or_else(|| (!self.ordered_ids.is_empty()).then_some(self.window_start));
        self.pending_focus = None;
        self.pending_moves = 0;
        self.wheel_accumulator = 0.0;
        self.last_wheel_at_ms = None;
        self.wheel_lock_until_ms = 0;
        self.last_wheel_direction = 0;
        self.transition_until_ms = 0;
        true
    }

    #[cfg(any(test, feature = "hydrate"))]
    fn record_wheel(&mut self, older_distance: f64, now_ms: u64) {
        let wheel_direction = older_distance.signum() as i32;
        if wheel_direction == 0
            || (now_ms < self.wheel_lock_until_ms && wheel_direction == self.last_wheel_direction)
        {
            return;
        }
        if self
            .last_wheel_at_ms
            .is_some_and(|last| now_ms.saturating_sub(last) > 160)
        {
            self.wheel_accumulator = 0.0;
        }
        self.last_wheel_at_ms = Some(now_ms);
        self.wheel_accumulator += older_distance;
        if self.wheel_accumulator.abs() < Self::WHEEL_TRIGGER_DISTANCE {
            return;
        }
        let direction: i32 = if self.wheel_accumulator > 0.0 { -1 } else { 1 };
        self.pending_moves = direction;
        self.wheel_lock_until_ms = now_ms.saturating_add(Self::WHEEL_DEBOUNCE_MS);
        self.last_wheel_direction = wheel_direction;
        self.wheel_accumulator = 0.0;
    }

    #[cfg(any(test, feature = "hydrate"))]
    fn next_target(&mut self) -> Option<usize> {
        if let Some(target) = self.pending_focus.take() {
            self.pending_moves = 0;
            return Some(self.clamp_window_start(target));
        }
        if self.pending_moves == 0 {
            return None;
        }
        let direction = self.pending_moves.signum();
        self.pending_moves = 0;
        Some(self.clamp_window_start(self.window_start.saturating_add_signed(direction as isize)))
    }

    #[cfg(any(test, feature = "hydrate"))]
    fn begin_next_move(&mut self, now_ms: u64) -> bool {
        if now_ms < self.transition_until_ms {
            return false;
        }
        loop {
            let Some(target) = self.next_target() else {
                return false;
            };
            if self.move_window_to(target) {
                self.transition_until_ms = now_ms.saturating_add(Self::TRANSITION_DURATION_MS);
                return true;
            }
        }
    }

    #[cfg(feature = "hydrate")]
    fn jump_to_pending(&mut self) {
        self.transition_until_ms = 0;
        while let Some(target) = self.next_target() {
            let _ = self.move_window_to(target);
        }
    }

    #[cfg(feature = "hydrate")]
    fn focus_index(&mut self, id: &str) -> bool {
        let Some(index) = self
            .ordered_ids
            .iter()
            .position(|candidate| candidate == id)
        else {
            return false;
        };
        let start = self.window_start;
        if (start..=start.saturating_add(1)).contains(&index) {
            return false;
        }
        let target = self.clamp_window_start(index.saturating_sub(1));
        self.pending_focus = Some(target);
        true
    }

    #[cfg(any(test, feature = "hydrate"))]
    fn move_window_to(&mut self, target: usize) -> bool {
        let target = self.clamp_window_start(target);
        let previous = self.window_start;
        if target == previous {
            if self.foreground_index.is_none() && !self.ordered_ids.is_empty() {
                self.foreground_index = Some(target);
            }
            return false;
        }
        self.foreground_index = if target < previous {
            Some(previous)
        } else if target == previous.saturating_add(1) {
            Some(previous.saturating_add(1))
        } else {
            Some(target)
        };
        self.window_start = target;
        true
    }

    #[cfg(any(test, feature = "hydrate"))]
    fn finish_transition(&mut self, now_ms: u64) -> bool {
        if self.transition_until_ms == 0 || now_ms < self.transition_until_ms {
            return false;
        }
        self.transition_until_ms = 0;
        true
    }
}

#[cfg(any(test, feature = "hydrate"))]
fn single_removal_index(previous_ids: &[String], next_ids: &[String]) -> Option<usize> {
    if previous_ids.len() != next_ids.len().saturating_add(1) {
        return None;
    }

    let mut next_index = 0;
    let mut removed_index = None;
    for (index, id) in previous_ids.iter().enumerate() {
        if next_ids.get(next_index) == Some(id) {
            next_index += 1;
        } else if removed_index.replace(index).is_some() {
            return None;
        }
    }
    if next_index != next_ids.len() {
        return None;
    }

    removed_index
}

#[cfg(any(test, feature = "hydrate"))]
fn normalize_wheel_delta(delta: f64, delta_mode: u32) -> f64 {
    match delta_mode {
        1 => delta * NotificationCarouselState::WHEEL_LINE_HEIGHT,
        2 => delta * NotificationCarouselState::WHEEL_PAGE_HEIGHT,
        _ => delta,
    }
}

#[derive(Clone)]
pub(super) struct NotificationCarouselController {
    state: RwSignal<NotificationCarouselState>,
    #[cfg(feature = "hydrate")]
    reduced_motion: RwSignal<bool>,
}

impl NotificationCarouselController {
    fn new(ordered_ids: Vec<String>, reduced_motion: RwSignal<bool>) -> Self {
        let state = RwSignal::new(NotificationCarouselState::new(ordered_ids));
        #[cfg(feature = "hydrate")]
        resize_notification_window(
            state.get_untracked().stack_height().saturating_add(8),
            false,
        );
        #[cfg(not(feature = "hydrate"))]
        let _ = reduced_motion;
        Self {
            state,
            #[cfg(feature = "hydrate")]
            reduced_motion,
        }
    }

    pub(super) fn visible_count(&self) -> usize {
        self.state.get().visible_count()
    }

    pub(super) fn has_more_top(&self) -> bool {
        self.state.get().has_more_top()
    }

    pub(super) fn has_more_bottom(&self) -> bool {
        self.state.get().has_more_bottom()
    }

    pub(super) fn visual_for(&self, id: &str) -> NotificationCardVisual {
        self.state.get().visual_for(id)
    }

    #[cfg(feature = "hydrate")]
    fn reconcile(&self, ordered_ids: Vec<String>) {
        let mut changed = false;
        self.state
            .update(|state| changed = state.reconcile(ordered_ids));
        if changed {
            resize_notification_window(
                self.state.get_untracked().stack_height().saturating_add(8),
                !self.reduced_motion.get_untracked(),
            );
        }
    }

    #[cfg(feature = "hydrate")]
    fn process_pending(&self) {
        self.process_pending_at(animation_clock_ms());
    }

    #[cfg(feature = "hydrate")]
    fn process_pending_at(&self, now_ms: u64) {
        if self.reduced_motion.get_untracked() {
            self.state
                .update(NotificationCarouselState::jump_to_pending);
            return;
        }
        self.state.update(|state| {
            let _ = state.begin_next_move(now_ms);
        });
    }

    #[cfg(feature = "hydrate")]
    fn handle_wheel(&self, event: web_sys::WheelEvent) {
        let delta_mode = event.delta_mode();
        let delta_x = normalize_wheel_delta(event.delta_x(), delta_mode);
        let delta_y = normalize_wheel_delta(event.delta_y(), delta_mode);
        if delta_y.abs() <= delta_x.abs() {
            return;
        }
        event.prevent_default();
        let now_ms = animation_clock_ms();
        let placement_factor = if notification_window_is_below() {
            1.0
        } else {
            -1.0
        };
        let delta_y = delta_y.clamp(-120.0, 120.0);
        self.state.update(|state| {
            state.record_wheel(
                delta_y * placement_factor * NotificationCarouselState::WHEEL_SCALE,
                now_ms,
            );
        });
        self.process_pending_at(now_ms);
    }

    #[cfg(feature = "hydrate")]
    fn handle_transition_end(&self, event: web_sys::TransitionEvent) {
        if event.property_name() != "top" {
            return;
        }
        let now_ms = animation_clock_ms();
        let mut finished = false;
        self.state
            .update(|state| finished = state.finish_transition(now_ms));
        if finished {
            self.process_pending_at(now_ms);
        }
    }

    #[cfg(feature = "hydrate")]
    fn focus_notification(&self, id: &str) {
        let mut found = false;
        self.state.update(|state| found = state.focus_index(id));
        if found {
            self.process_pending();
        }
    }
}

#[cfg(feature = "hydrate")]
fn notification_window_is_below() -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
        .and_then(|root| root.get_attribute("data-notification-placement"))
        .is_some_and(|placement| placement == "below")
}

fn notifications_by_display_order(
    mut notifications: Vec<PetNotificationPresentation>,
) -> Vec<PetNotificationPresentation> {
    notifications.sort_by(|left, right| {
        left.occurred_at_ms
            .cmp(&right.occurred_at_ms)
            .then_with(|| left.activation_id.cmp(&right.activation_id))
    });
    notifications
}

fn notifications_by_keyboard_order(
    notifications: Vec<PetNotificationPresentation>,
) -> Vec<PetNotificationPresentation> {
    let mut notifications = notifications_by_display_order(notifications);
    notifications.reverse();
    notifications
}

#[component]
fn NotificationCard(
    notification: PetNotificationPresentation,
    wall_clock: RwSignal<u64>,
    carousel: NotificationCarouselController,
) -> AnyView {
    let activation_id = notification.activation_id;
    let kind = notification.kind;
    let project_label = notification
        .project_label
        .unwrap_or_else(|| "Session".to_owned());
    let summary = notification.summary;
    let occurred_at_ms = notification.occurred_at_ms;
    let unread = notification.unread;
    let status = match kind {
        PetNotificationKind::Attention | PetNotificationKind::Failure => Some(
            view! {
                <span
                    class="notification-status"
                    class:notification-status-attention=kind == PetNotificationKind::Attention
                    class:notification-status-failure=kind == PetNotificationKind::Failure
                    role="img"
                    aria-label=format!("{} notification", notification_kind_label(kind))
                >
                    <NotificationStatusIcon kind/>
                </span>
            }
            .into_any(),
        ),
        PetNotificationKind::Completion => None,
    };
    #[cfg(feature = "hydrate")]
    let controls = {
        let activate_id = activation_id.clone();
        let dismiss_id = activation_id.clone();
        let activate_focus_id = activation_id.clone();
        let dismiss_focus_id = activation_id.clone();
        let activate_carousel = carousel.clone();
        let dismiss_carousel = carousel.clone();
        view! {
            <button
                class="notification-activate"
                type="button"
                aria-label=format!("Open {} notification for {project_label}", notification_kind_label(kind))
                tabindex="0"
                on:focus=move |_| activate_carousel.focus_notification(&activate_focus_id)
                on:click=move |_| activate_native_notification(&activate_id)
            >
                <NotificationOpenIcon/>
            </button>
            <button
                class="notification-dismiss"
                type="button"
                aria-label=format!("Dismiss {} notification for {project_label}", notification_kind_label(kind))
                tabindex="0"
                on:focus=move |_| dismiss_carousel.focus_notification(&dismiss_focus_id)
                on:click=move |_| dismiss_native_notification(&dismiss_id)
            >
                <NotificationDismissIcon/>
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
            <NotificationOpenIcon/>
        </button>
        <button
            class="notification-dismiss"
            type="button"
            aria-label=format!("Dismiss {} notification for {project_label}", notification_kind_label(kind))
        >
            <NotificationDismissIcon/>
        </button>
    };
    let top_carousel = carousel.clone();
    let top_id = activation_id.clone();
    let bottom_carousel = carousel.clone();
    let bottom_id = activation_id.clone();
    let top_behind_carousel = carousel.clone();
    let top_behind_id = activation_id.clone();
    let bottom_behind_carousel = carousel.clone();
    let bottom_behind_id = activation_id.clone();
    let current_carousel = carousel.clone();
    let current_id = activation_id.clone();
    let foreground_carousel = carousel.clone();
    let foreground_id = activation_id.clone();
    view! {
        <article
            class="notification-card"
            class:notification-unread=unread
            class:notification-card-no-status=kind == PetNotificationKind::Completion
            class:notification-card-top=move || {
                top_carousel.visual_for(&top_id).role == NotificationCardRole::Top
            }
            class:notification-card-bottom=move || {
                bottom_carousel.visual_for(&bottom_id).role == NotificationCardRole::Bottom
            }
            class:notification-card-top-behind=move || {
                top_behind_carousel.visual_for(&top_behind_id).role
                    == NotificationCardRole::TopBehind
            }
            class:notification-card-bottom-behind=move || {
                bottom_behind_carousel.visual_for(&bottom_behind_id).role
                    == NotificationCardRole::BottomBehind
            }
            class:notification-card-current=move || current_carousel
                .visual_for(&current_id)
                .current
            class:notification-card-foreground=move || foreground_carousel
                .visual_for(&foreground_id)
                .foreground
            data-notification-id=activation_id
            data-notification-kind=kind.as_str()
        >
            <div class="notification-card-body">
                {status}
                <div class="notification-content">
                    <div class="notification-heading">
                        <strong class="notification-project">{project_label}</strong>
                        <time class="notification-time">{move || relative_time_label(occurred_at_ms, wall_clock.get())}</time>
                    </div>
                    <p class="notification-summary">{summary}</p>
                </div>
                <div class="notification-controls">{controls}</div>
            </div>
        </article>
    }
    .into_any()
}

#[component]
fn NotificationStatusIcon(kind: PetNotificationKind) -> impl IntoView {
    match kind {
        PetNotificationKind::Attention => view! {
            <svg
                class="notification-status-icon"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
            >
                <path d="m21.73 18-8-14a2 2 0 0 0-3.46 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/>
                <path d="M12 9v4"/>
                <path d="M12 17h.01"/>
            </svg>
        }
        .into_any(),
        PetNotificationKind::Completion => view! {
            <svg
                class="notification-status-icon"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
            >
                <path d="M20 6 9 17l-5-5"/>
            </svg>
        }
        .into_any(),
        PetNotificationKind::Failure => view! {
            <svg
                class="notification-status-icon"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
            >
                <path d="m6 6 12 12"/>
                <path d="m18 6-12 12"/>
            </svg>
        }
        .into_any(),
    }
}

#[component]
fn NotificationOpenIcon() -> AnyView {
    view! {
        <svg
            class="notification-action-icon"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
        >
            <path d="m7 17 10-10"/>
            <path d="M7 7h10v10"/>
        </svg>
    }
    .into_any()
}

#[component]
fn NotificationDismissIcon() -> AnyView {
    view! {
        <svg
            class="notification-action-icon"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
        >
            <path d="m7 7 10 10"/>
            <path d="m17 7-10 10"/>
        </svg>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_with_the_newest_pair() {
        let state = NotificationCarouselState::new(vec![
            "oldest".to_owned(),
            "middle".to_owned(),
            "newest".to_owned(),
            "latest".to_owned(),
        ]);

        assert_eq!(state.window_start, 2);
        assert_eq!(state.visible_count(), 2);
        assert_eq!(state.stack_height(), 148);
        assert_eq!(state.visual_for("newest").role, NotificationCardRole::Top);
        assert_eq!(
            state.visual_for("latest").role,
            NotificationCardRole::Bottom
        );
        assert!(state.visual_for("newest").foreground);
        assert_eq!(
            state.visual_for("middle").role,
            NotificationCardRole::TopBehind
        );
        assert_eq!(state.visual_for("oldest"), NotificationCardVisual::HIDDEN);
        assert!(state.has_more_top());
        assert!(!state.has_more_bottom());
    }

    #[test]
    fn preserves_the_current_window_when_removing_one_notification() {
        let mut state = NotificationCarouselState::new(vec![
            "oldest".to_owned(),
            "middle".to_owned(),
            "newest".to_owned(),
            "latest".to_owned(),
            "latest-extra".to_owned(),
        ]);
        state.move_window_to(2);
        state.move_window_to(1);

        assert_eq!(state.window_start, 1);
        assert!(state.visual_for("newest").foreground);

        assert!(state.reconcile(vec![
            "oldest".to_owned(),
            "middle".to_owned(),
            "latest".to_owned(),
            "latest-extra".to_owned(),
        ]));

        assert_eq!(state.window_start, 1);
        assert_eq!(state.visual_for("middle").role, NotificationCardRole::Top);
        assert_eq!(
            state.visual_for("latest").role,
            NotificationCardRole::Bottom
        );
        assert!(state.visual_for("middle").foreground);
        assert!(state.has_more_top());
        assert!(state.has_more_bottom());
    }

    #[test]
    fn shifts_the_current_window_when_removing_an_older_notification() {
        let mut state = NotificationCarouselState::new(vec![
            "oldest".to_owned(),
            "middle".to_owned(),
            "newest".to_owned(),
            "latest".to_owned(),
        ]);
        state.move_window_to(1);

        assert!(state.reconcile(vec![
            "middle".to_owned(),
            "newest".to_owned(),
            "latest".to_owned(),
        ]));

        assert_eq!(state.window_start, 0);
        assert_eq!(state.visual_for("middle").role, NotificationCardRole::Top);
        assert_eq!(
            state.visual_for("newest").role,
            NotificationCardRole::Bottom
        );
        assert!(state.visual_for("newest").foreground);
        assert!(!state.has_more_top());
        assert!(state.has_more_bottom());
    }

    #[test]
    fn fills_the_older_side_when_removing_the_current_window_start() {
        let mut state = NotificationCarouselState::new(vec![
            "d".to_owned(),
            "c".to_owned(),
            "b".to_owned(),
            "a".to_owned(),
        ]);
        state.move_window_to(1);

        assert_eq!(state.visual_for("b").role, NotificationCardRole::Bottom);
        assert_eq!(state.visual_for("c").role, NotificationCardRole::Top);

        assert!(state.reconcile(vec!["d".to_owned(), "b".to_owned(), "a".to_owned(),]));

        assert_eq!(state.window_start, 0);
        assert_eq!(state.visual_for("d").role, NotificationCardRole::Top);
        assert_eq!(state.visual_for("b").role, NotificationCardRole::Bottom);
        assert_eq!(
            state.visual_for("a").role,
            NotificationCardRole::BottomBehind
        );

        assert!(state.reconcile(vec!["b".to_owned(), "a".to_owned()]));

        assert_eq!(state.window_start, 0);
        assert_eq!(state.visual_for("b").role, NotificationCardRole::Top);
        assert_eq!(state.visual_for("a").role, NotificationCardRole::Bottom);
    }

    #[test]
    fn keeps_a_single_notification_at_the_bottom_of_the_sorted_stack() {
        let mut state = NotificationCarouselState::new(vec![
            "oldest".to_owned(),
            "middle".to_owned(),
            "newest".to_owned(),
        ]);

        assert!(state.reconcile(vec!["oldest".to_owned(), "middle".to_owned()]));
        assert_eq!(
            state.visual_for("middle").role,
            NotificationCardRole::Bottom
        );

        assert!(state.reconcile(vec!["oldest".to_owned()]));

        assert_eq!(
            state.visual_for("oldest").role,
            NotificationCardRole::Bottom
        );
        assert_eq!(state.stack_height(), 82);
    }

    #[test]
    fn keeps_a_single_notification_at_the_bottom_after_reverse_removal() {
        let mut state = NotificationCarouselState::new(vec![
            "oldest".to_owned(),
            "middle".to_owned(),
            "newest".to_owned(),
        ]);

        assert!(state.reconcile(vec!["oldest".to_owned(), "newest".to_owned()]));
        assert_eq!(state.visual_for("oldest").role, NotificationCardRole::Top);

        assert!(state.reconcile(vec!["newest".to_owned()]));

        assert_eq!(
            state.visual_for("newest").role,
            NotificationCardRole::Bottom
        );
        assert_eq!(state.stack_height(), 82);
    }

    #[test]
    fn keeps_shared_card_in_front_during_continuous_move() {
        let mut state = NotificationCarouselState::new(vec![
            "oldest".to_owned(),
            "middle".to_owned(),
            "newest".to_owned(),
            "latest".to_owned(),
        ]);

        state.record_wheel(60.0, 100);
        assert_eq!(state.pending_moves, -1);
        assert!(state.begin_next_move(100));
        assert_eq!(state.window_start, 1);
        assert_eq!(state.visual_for("middle").role, NotificationCardRole::Top);
        assert_eq!(
            state.visual_for("newest").role,
            NotificationCardRole::Bottom
        );
        assert_eq!(
            state.visual_for("latest").role,
            NotificationCardRole::BottomBehind
        );
        assert_eq!(
            state.visual_for("oldest").role,
            NotificationCardRole::TopBehind
        );
        assert!(!state.visual_for("middle").foreground);
        assert!(state.visual_for("newest").foreground);
        assert!(state.has_more_top());
        assert!(state.has_more_bottom());
    }

    #[test]
    fn ignores_repeated_same_direction_input_within_debounce_window() {
        let mut state = NotificationCarouselState::new(vec![
            "oldest".to_owned(),
            "middle".to_owned(),
            "newest".to_owned(),
        ]);

        state.record_wheel(60.0, 100);
        assert!(state.begin_next_move(100));
        state.record_wheel(60.0, 120);
        assert_eq!(state.pending_moves, 0);
        state.record_wheel(-60.0, 120);
        assert_eq!(state.pending_moves, 1);
        assert!(!state.begin_next_move(120));
        assert_eq!(state.window_start, 0);
        assert!(state.finish_transition(560));
        assert!(state.begin_next_move(560));
        assert_eq!(state.window_start, 1);
    }

    #[test]
    fn normalizes_wheel_delta_modes_before_thresholding() {
        assert_eq!(normalize_wheel_delta(12.0, 0), 12.0);
        assert_eq!(normalize_wheel_delta(3.0, 1), 48.0);
        assert_eq!(normalize_wheel_delta(-1.0, 2), -148.0);
    }
}
