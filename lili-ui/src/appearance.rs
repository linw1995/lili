#[cfg(any(test, feature = "hydrate"))]
use std::time::Duration;

use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use lili_core::AppearanceSelectionRequest;
#[cfg(feature = "hydrate")]
use lili_core::PetId;
use lili_core::{AppearanceView, PetNotificationKind, PetNotificationPresentation};
use lili_pet::{AnimationScheduler, FrameDescriptor, PreviewScene};

use super::notification_carousel::NotificationPreviewCard;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
function appearanceWindowInvoke(command) {
  const invoke = window.__TAURI_INTERNALS__?.invoke;
  const label = window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label;
  if (!invoke || !label) return;
  void invoke(`plugin:window|${command}`, { label }).catch(() => {});
}

export function closeAppearanceWindow() {
  appearanceWindowInvoke('close');
}

export function appearanceWindowIsNative() {
  return Boolean(window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label);
}

export function appearanceWindowIsVisible() {
  return window.__LILI_APPEARANCE_VISIBLE__ === true;
}

export function installAppearanceSelectionBoundary() {
  const root = document.getElementById('lili-appearance');
  if (!root || root.dataset.appearanceSelectionBoundary === 'true') return;

  let selectionScope = null;
  const scopeFor = (target) => {
    const element = target instanceof Element ? target : target?.parentElement;
    const scope = element?.closest?.('.appearance-selectable') ?? null;
    return scope && root.contains(scope) ? scope : null;
  };

  const rememberScope = (event) => {
    const scope = scopeFor(event.target);
    if (scope || event.type === 'pointerdown') {
      selectionScope = scope;
    }
  };

  const boundSelection = () => {
    const selection = window.getSelection();
    if (!selection || !selectionScope || selection.rangeCount === 0 || selection.isCollapsed) {
      return;
    }
    if (!root.contains(selection.anchorNode) || !root.contains(selection.focusNode)) {
      selectionScope = null;
      return;
    }

    // Desktop WebViews do not consistently support user-select: contain, so clamp the live range here.
    const range = selection.getRangeAt(0);
    const scopeRange = document.createRange();
    scopeRange.selectNodeContents(selectionScope);
    const boundedRange = range.cloneRange();
    let changed = false;

    if (range.compareBoundaryPoints(Range.START_TO_START, scopeRange) < 0) {
      boundedRange.setStart(scopeRange.startContainer, scopeRange.startOffset);
      changed = true;
    }
    if (range.compareBoundaryPoints(Range.END_TO_END, scopeRange) > 0) {
      boundedRange.setEnd(scopeRange.endContainer, scopeRange.endOffset);
      changed = true;
    }

    if (changed) {
      selection.removeAllRanges();
      if (!boundedRange.collapsed) {
        selection.addRange(boundedRange);
      }
    }
  };

  root.addEventListener('pointerdown', rememberScope, true);
  root.addEventListener('selectstart', rememberScope, true);
  document.addEventListener('selectionchange', boundSelection);
  root.dataset.appearanceSelectionBoundary = 'true';
}
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_name = closeAppearanceWindow)]
    fn close_appearance_window();

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = appearanceWindowIsNative)]
    fn appearance_window_is_native() -> bool;

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = appearanceWindowIsVisible)]
    fn appearance_window_is_visible() -> bool;

    #[wasm_bindgen::prelude::wasm_bindgen(js_name = installAppearanceSelectionBoundary)]
    fn install_appearance_selection_boundary_js();
}

#[cfg(not(feature = "hydrate"))]
fn close_appearance_window() {}

#[cfg(feature = "hydrate")]
pub(crate) fn install_appearance_selection_boundary() {
    install_appearance_selection_boundary_js();
}

#[cfg(feature = "hydrate")]
struct AppearanceClock {
    window: web_sys::Window,
    interval_id: i32,
    _callback: wasm_bindgen::closure::Closure<dyn FnMut()>,
}

#[cfg(feature = "hydrate")]
struct AppearanceRefreshClock {
    window: web_sys::Window,
    interval_id: i32,
    _callback: wasm_bindgen::closure::Closure<dyn FnMut()>,
}

#[cfg(feature = "hydrate")]
struct AppearanceLifecycle {
    _visibility_callback: Option<wasm_bindgen::closure::Closure<dyn FnMut()>>,
    _shown_callback: Option<wasm_bindgen::closure::Closure<dyn FnMut()>>,
    _hidden_callback: Option<wasm_bindgen::closure::Closure<dyn FnMut()>>,
}

#[cfg(feature = "hydrate")]
thread_local! {
    static APPEARANCE_CLOCK: std::cell::RefCell<Option<AppearanceClock>> =
        const { std::cell::RefCell::new(None) };
    static APPEARANCE_REFRESH_CLOCK: std::cell::RefCell<Option<AppearanceRefreshClock>> =
        const { std::cell::RefCell::new(None) };
    static APPEARANCE_LIFECYCLE: std::cell::RefCell<Option<AppearanceLifecycle>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
pub fn AppearancePage(appearance: AppearanceView) -> impl IntoView {
    let appearance = RwSignal::new(appearance);
    let selection_error = RwSignal::new(None::<String>);
    #[cfg(feature = "hydrate")]
    let selection_in_flight = RwSignal::new(false);
    let preview = RwSignal::new(AppearancePreviewController::new(PreviewScene::Idle));
    let preview_frame = RwSignal::new(preview.get_untracked().frame());
    let preview_wall_clock = RwSignal::new(0_u64);
    #[cfg(feature = "hydrate")]
    let preview_reduced_motion = RwSignal::new(super::system_prefers_reduced_motion());
    #[cfg(not(feature = "hydrate"))]
    let preview_reduced_motion = RwSignal::new(false);
    #[cfg(feature = "hydrate")]
    start_appearance_lifecycle(
        appearance,
        selection_in_flight,
        preview,
        preview_frame,
        preview_reduced_motion,
    );
    let initial_appearance = appearance.get_untracked();
    let serialized_appearance = serde_json::to_string(&initial_appearance).unwrap_or_default();
    let selected_asset_url = move || {
        let current = appearance.get();
        current
            .selected_pet_id
            .as_ref()
            .and_then(|selected| current.pets.iter().find(|pet| &pet.id == selected))
            .map(|pet| format!("/pet-assets/{}", pet.asset_id))
            .unwrap_or_default()
    };
    #[cfg(feature = "hydrate")]
    let pet_items = view! {
        <For
            each=move || appearance.get().pets.clone()
            key=|pet| pet.asset_id.clone()
            children=move |pet| {
                let pet_id = pet.id.clone();
                let pet_id_value = pet.id.as_str().to_owned();
                let asset_url = format!("/pet-assets/{}", pet.asset_id);
                let selected_pet_id = pet_id.clone();
                let selected = Memo::new(move |_| {
                    appearance.get().selected_pet_id.as_ref() == Some(&selected_pet_id)
                });
                let select_id = pet_id.clone();
                view! {
                    <button
                        class="appearance-pet-item"
                        class:appearance-pet-selected=move || selected.get()
                        type="button"
                        role="option"
                        aria-selected=move || selected.get().to_string()
                        data-pet-id=pet_id_value
                        on:click=move |_| {
                            if selection_in_flight.get_untracked() {
                                return;
                            }
                            request_pet_selection(
                                appearance,
                                selection_error,
                                selection_in_flight,
                                select_id.clone(),
                            );
                        }
                    >
                        <span class="appearance-pet-thumb">
                            <img
                                class="appearance-pet-thumb-atlas"
                                src=asset_url.clone()
                                alt=""
                                aria-hidden="true"
                                draggable="false"
                            />
                        </span>
                        <span class="appearance-pet-item-copy appearance-selectable">
                            <strong>{pet.display_name}</strong>
                            <span>{move || if selected.get() { "Selected" } else { "Installed package" }}</span>
                        </span>
                        <span class="appearance-pet-item-check" aria-hidden="true">
                            {move || if selected.get() { "✓" } else { "" }}
                        </span>
                    </button>
                }
            }
        />
    };
    #[cfg(not(feature = "hydrate"))]
    let pet_items = initial_appearance
        .pets
        .iter()
        .cloned()
        .map(|pet| {
            let selected = initial_appearance.selected_pet_id.as_ref() == Some(&pet.id);
            appearance_pet_item_static(pet, selected)
        })
        .collect_view();
    let scene_items = PreviewScene::all()
        .iter()
        .copied()
        .map(|scene| appearance_scene_button(scene, preview, preview_frame))
        .collect_view();

    view! {
        <main
            id="lili-appearance"
            class="appearance-surface"
            data-ssr-marker="appearance-ready"
            data-appearance-view=serialized_appearance
        >
            <div class="appearance-window-frame">
                <header class="appearance-topbar" data-tauri-drag-region="deep">
                    <div class="appearance-topbar-leading">
                        <div class="appearance-window-controls" aria-label="Window controls">
                            <button
                                class="appearance-window-control appearance-window-control-close"
                                type="button"
                                aria-label="Close Appearance window"
                                title="Close"
                                on:click=move |_| close_appearance_window()
                            >
                                <span aria-hidden="true">"×"</span>
                            </button>
                        </div>
                        <div class="appearance-brand">
                            <span class="appearance-brand-copy appearance-selectable"><strong>"Lili"</strong><span>"Pet Studio"</span></span>
                        </div>
                    </div>
                </header>

                <div class="appearance-body">
                <nav class="appearance-sidebar" aria-label="Settings sections">
                    <button class="appearance-nav-button" type="button" aria-current="page">
                        <span aria-hidden="true">"🐾"</span><span>"Pet"</span>
                    </button>
                </nav>

                <section class="appearance-main" aria-labelledby="appearance-heading">
                    <div class="appearance-page-heading">
                        <h1 id="appearance-heading" class="appearance-selectable">"Appearance"</h1>
                    </div>

                    <div class="appearance-workbench">
                        <section class="appearance-preview-panel" aria-label="Appearance preview">
                            <div class="appearance-panel-heading">
                                <div class="appearance-panel-heading-copy">
                                    <strong>"Live preview"</strong>
                                </div>
                            </div>

                            <div class="appearance-scene-picker">
                                <div class="appearance-scene-heading">
                                    <strong class="appearance-selectable">"Scene"</strong>
                                </div>
                                <div class="appearance-scene-buttons" role="group" aria-label="Preview scenes">
                                    {scene_items}
                                </div>
                            </div>

                            <div class="appearance-stage" aria-live="polite">
                                <div
                                    class="appearance-scene"
                                    id="appearance-scene"
                                    data-placement=move || scene_placement(preview.get().scene())
                                    data-scene=move || scene_token(preview.get().scene())
                                    data-animation=move || animation_token(preview.get().scene().spec().animation())
                                >
                                    <div class="appearance-notifications" id="appearance-notifications">
                                        <Show
                                            when=move || preview.get().scene().spec().notification().is_some()
                                            fallback=|| view! {
                                                <div class="appearance-preview-empty">"No pending notifications"</div>
                                            }
                                        >
                                            {move || {
                                                let notification = preview_notification(preview.get().scene())
                                                    .expect("notification preview scene must provide a notification");
                                                view! {
                                                    <NotificationPreviewCard
                                                        notification
                                                        wall_clock=preview_wall_clock
                                                        reduced_motion=preview_reduced_motion
                                                    />
                                                }
                                            }}
                                        </Show>
                                    </div>
                                    <div class="appearance-pet" id="appearance-pet">
                                        <img
                                            class="appearance-pet-atlas"
                                            src=selected_asset_url
                                            alt=""
                                            aria-hidden="true"
                                            draggable="false"
                                            data-frame-row=move || preview_frame.get().row()
                                            data-frame-column=move || preview_frame.get().column()
                                            style:animation="none"
                                            style:transform=move || frame_transform(preview_frame.get())
                                        />
                                        <span class="appearance-pet-tag">
                                            {move || scene_label(preview.get().scene())}
                                        </span>
                                        </div>
                                    </div>
                            </div>
                        </section>

                        <aside class="appearance-pet-panel" aria-label="Pet list">
                            <h2 class="appearance-selectable">"Pet"</h2>
                            <div class="appearance-pet-list-heading">
                                <span>"Installed pets"</span>
                                <span>{move || format!("{} available", appearance.get().pets.len())}</span>
                            </div>
                            <div class="appearance-pet-list" role="listbox" aria-label="Installed pets">
                                {pet_items}
                            </div>
                            <Show when=move || selection_error.get().is_some()>
                                <div class="appearance-status-line">
                                    <span class="appearance-status-dot" aria-hidden="true"></span>
                                    <small role="alert" class="appearance-selection-error appearance-selectable">
                                        {move || selection_error.get().unwrap_or_else(|| "Pet selection failed".to_owned())}
                                    </small>
                                </div>
                            </Show>
                        </aside>
                    </div>
                </section>
            </div>
            </div>
        </main>
    }
}

#[cfg(not(feature = "hydrate"))]
fn appearance_pet_item_static(pet: lili_core::AppearancePetView, selected: bool) -> impl IntoView {
    let asset_url = format!("/pet-assets/{}", pet.asset_id);
    view! {
        <button
            class="appearance-pet-item"
            class:appearance-pet-selected=selected
            type="button"
            role="option"
            aria-selected=selected.to_string()
            data-pet-id=pet.id.as_str().to_owned()
        >
            <span class="appearance-pet-thumb">
                <img
                    class="appearance-pet-thumb-atlas"
                    src=asset_url
                    alt=""
                    aria-hidden="true"
                    draggable="false"
                />
            </span>
            <span class="appearance-pet-item-copy appearance-selectable">
                <strong>{pet.display_name}</strong>
                <span>{if selected { "Selected" } else { "Installed package" }}</span>
            </span>
            <span class="appearance-pet-item-check" aria-hidden="true">
                {if selected { "✓" } else { "" }}
            </span>
        </button>
    }
}

fn appearance_scene_button(
    scene: PreviewScene,
    preview: RwSignal<AppearancePreviewController>,
    preview_frame: RwSignal<FrameDescriptor>,
) -> impl IntoView {
    view! {
        <button
            class="appearance-scene-button"
            class:appearance-scene-selected=move || preview.get().scene() == scene
            type="button"
            aria-pressed=move || (preview.get().scene() == scene).to_string()
            data-scene=scene_token(scene)
            on:click=move |_| {
                let mut frame = preview.get_untracked().frame();
                preview.update(|preview| frame = preview.select(scene));
                preview_frame.set(frame);
            }
        >
            <span class="appearance-scene-glyph" aria-hidden="true">{scene_glyph(scene)}</span>
            <span>{scene_label(scene)}</span>
        </button>
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AppearancePreviewController {
    scene: PreviewScene,
    scheduler: AnimationScheduler,
}

impl AppearancePreviewController {
    fn new(scene: PreviewScene) -> Self {
        Self {
            scene,
            scheduler: AnimationScheduler::new(scene.spec().animation()),
        }
    }

    const fn scene(self) -> PreviewScene {
        self.scene
    }

    fn frame(self) -> FrameDescriptor {
        self.scheduler.current_frame()
    }

    fn select(&mut self, scene: PreviewScene) -> FrameDescriptor {
        self.scene = scene;
        self.scheduler = AnimationScheduler::new(scene.spec().animation());
        self.frame()
    }

    #[cfg(any(test, feature = "hydrate"))]
    fn advance(&mut self, delta: Duration) -> FrameDescriptor {
        self.scheduler.advance(delta)
    }

    #[cfg(any(test, feature = "hydrate"))]
    fn tick(&mut self, delta: Duration, reduced_motion: bool) -> FrameDescriptor {
        if reduced_motion {
            self.frame()
        } else {
            self.advance(delta)
        }
    }
}

fn frame_transform(frame: FrameDescriptor) -> String {
    format!(
        "translate(-{}px,-{}px)",
        u32::from(frame.column()) * lili_pet::CELL_WIDTH,
        u32::from(frame.row()) * lili_pet::CELL_HEIGHT,
    )
}

const fn animation_token(animation: lili_pet::AnimationState) -> &'static str {
    match animation {
        lili_pet::AnimationState::Idle => "idle",
        lili_pet::AnimationState::RunningRight => "running-right",
        lili_pet::AnimationState::RunningLeft => "running-left",
        lili_pet::AnimationState::Waving => "waving",
        lili_pet::AnimationState::Jumping => "jumping",
        lili_pet::AnimationState::Failed => "failed",
        lili_pet::AnimationState::Waiting => "waiting",
        lili_pet::AnimationState::Running => "running",
        lili_pet::AnimationState::Review => "review",
    }
}

const fn scene_placement(_scene: PreviewScene) -> &'static str {
    "above"
}

fn preview_notification(scene: PreviewScene) -> Option<PetNotificationPresentation> {
    let (kind, project_label, summary) = match scene {
        PreviewScene::Review => (
            PetNotificationKind::Completion,
            "Review",
            "Task completed successfully.",
        ),
        PreviewScene::Attention => (
            PetNotificationKind::Attention,
            "Attention",
            "A task needs your attention.",
        ),
        PreviewScene::Failed => (
            PetNotificationKind::Failure,
            "Failed",
            "The task could not be completed.",
        ),
        PreviewScene::Waiting => (
            PetNotificationKind::Attention,
            "Waiting",
            "Waiting for the next task update.",
        ),
        PreviewScene::Idle | PreviewScene::Running | PreviewScene::Click => return None,
    };
    Some(PetNotificationPresentation {
        activation_id: format!("appearance-preview-{}", scene_token(scene)),
        kind,
        project_label: Some(project_label.to_owned()),
        summary: summary.to_owned(),
        summary_truncated: false,
        summary_redacted: false,
        occurred_at_ms: 0,
        unread: false,
    })
}

const fn scene_token(scene: PreviewScene) -> &'static str {
    match scene {
        PreviewScene::Idle => "idle",
        PreviewScene::Running => "running",
        PreviewScene::Review => "review",
        PreviewScene::Attention => "attention",
        PreviewScene::Failed => "failed",
        PreviewScene::Waiting => "waiting",
        PreviewScene::Click => "click",
    }
}

const fn scene_label(scene: PreviewScene) -> &'static str {
    match scene {
        PreviewScene::Idle => "Idle",
        PreviewScene::Running => "Running",
        PreviewScene::Review => "Review",
        PreviewScene::Attention => "Attention",
        PreviewScene::Failed => "Failed",
        PreviewScene::Waiting => "Waiting",
        PreviewScene::Click => "Click",
    }
}

const fn scene_glyph(scene: PreviewScene) -> &'static str {
    match scene {
        PreviewScene::Idle => "●",
        PreviewScene::Running => "→",
        PreviewScene::Review => "◉",
        PreviewScene::Attention => "!",
        PreviewScene::Failed => "×",
        PreviewScene::Waiting => "◷",
        PreviewScene::Click => "✦",
    }
}

#[cfg(feature = "hydrate")]
fn start_appearance_lifecycle(
    appearance: RwSignal<AppearanceView>,
    selection_in_flight: RwSignal<bool>,
    preview: RwSignal<AppearancePreviewController>,
    preview_frame: RwSignal<FrameDescriptor>,
    preview_reduced_motion: RwSignal<bool>,
) {
    use wasm_bindgen::{JsCast, closure::Closure};

    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    if APPEARANCE_LIFECYCLE.with(|lifecycle| lifecycle.borrow().is_some()) {
        return;
    }

    let mut lifecycle = AppearanceLifecycle {
        _visibility_callback: None,
        _shown_callback: None,
        _hidden_callback: None,
    };
    if appearance_window_is_native() {
        let shown_callback = Closure::<dyn FnMut()>::new(move || {
            start_appearance_preview_clock(preview, preview_frame, preview_reduced_motion);
            start_appearance_selection_refresh(appearance, selection_in_flight);
        });
        let hidden_callback = Closure::<dyn FnMut()>::new(move || {
            stop_appearance_preview_clock();
            stop_appearance_selection_refresh();
        });
        let _ = window.add_event_listener_with_callback(
            "lili-appearance-shown",
            shown_callback.as_ref().unchecked_ref(),
        );
        let _ = window.add_event_listener_with_callback(
            "lili-appearance-hidden",
            hidden_callback.as_ref().unchecked_ref(),
        );
        lifecycle._shown_callback = Some(shown_callback);
        lifecycle._hidden_callback = Some(hidden_callback);
        if appearance_window_is_visible() {
            start_appearance_preview_clock(preview, preview_frame, preview_reduced_motion);
            start_appearance_selection_refresh(appearance, selection_in_flight);
        }
    } else {
        let visibility_callback = Closure::<dyn FnMut()>::new({
            let document = document.clone();
            move || {
                if document.hidden() {
                    stop_appearance_preview_clock();
                    stop_appearance_selection_refresh();
                } else {
                    start_appearance_preview_clock(preview, preview_frame, preview_reduced_motion);
                    start_appearance_selection_refresh(appearance, selection_in_flight);
                }
            }
        });
        let _ = document.add_event_listener_with_callback(
            "visibilitychange",
            visibility_callback.as_ref().unchecked_ref(),
        );
        lifecycle._visibility_callback = Some(visibility_callback);
        if !document.hidden() {
            start_appearance_preview_clock(preview, preview_frame, preview_reduced_motion);
            start_appearance_selection_refresh(appearance, selection_in_flight);
        }
    }
    APPEARANCE_LIFECYCLE.with(|stored| {
        stored.borrow_mut().replace(lifecycle);
    });
}

#[cfg(feature = "hydrate")]
fn stop_appearance_preview_clock() {
    APPEARANCE_CLOCK.with(|clock| {
        if let Some(clock) = clock.borrow_mut().take() {
            clock.window.clear_interval_with_handle(clock.interval_id);
        }
    });
}

#[cfg(feature = "hydrate")]
fn stop_appearance_selection_refresh() {
    APPEARANCE_REFRESH_CLOCK.with(|clock| {
        if let Some(clock) = clock.borrow_mut().take() {
            clock.window.clear_interval_with_handle(clock.interval_id);
        }
    });
}

#[cfg(feature = "hydrate")]
fn start_appearance_preview_clock(
    preview: RwSignal<AppearancePreviewController>,
    preview_frame: RwSignal<FrameDescriptor>,
    preview_reduced_motion: RwSignal<bool>,
) {
    use std::cell::Cell;

    use wasm_bindgen::{JsCast, closure::Closure};

    let Some(window) = web_sys::window() else {
        return;
    };
    let last_tick = Cell::new(super::animation_clock_ms());
    let callback = Closure::<dyn FnMut()>::new(move || {
        let now = super::animation_clock_ms();
        let elapsed_ms = now.saturating_sub(last_tick.replace(now));
        let reduced_motion = super::system_prefers_reduced_motion();
        if preview_reduced_motion.get_untracked() != reduced_motion {
            preview_reduced_motion.set(reduced_motion);
        }
        let mut frame = preview.get_untracked().frame();
        preview.update(|preview| {
            frame = preview.tick(Duration::from_millis(elapsed_ms), reduced_motion)
        });
        preview_frame.set(frame);
    });
    let Ok(interval_id) = window.set_interval_with_callback_and_timeout_and_arguments_0(
        callback.as_ref().unchecked_ref(),
        16,
    ) else {
        return;
    };
    APPEARANCE_CLOCK.with(|clock| {
        if let Some(previous) = clock.borrow_mut().replace(AppearanceClock {
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
fn start_appearance_selection_refresh(
    appearance: RwSignal<AppearanceView>,
    selection_in_flight: RwSignal<bool>,
) {
    use std::{cell::Cell, rc::Rc};

    use wasm_bindgen::{JsCast, closure::Closure};

    let Some(window) = web_sys::window() else {
        return;
    };
    let request_in_flight = Rc::new(Cell::new(false));
    let refresh_window = window.clone();
    let callback = Closure::<dyn FnMut()>::new({
        let request_in_flight = Rc::clone(&request_in_flight);
        move || {
            if selection_in_flight.get_untracked() || request_in_flight.replace(true) {
                return;
            }
            let window = refresh_window.clone();
            let request_in_flight = Rc::clone(&request_in_flight);
            wasm_bindgen_futures::spawn_local(async move {
                if let Ok(next) = fetch_appearance_view(&window).await {
                    appearance.set(next);
                }
                request_in_flight.set(false);
            });
        }
    });
    let Ok(interval_id) = window.set_interval_with_callback_and_timeout_and_arguments_0(
        callback.as_ref().unchecked_ref(),
        500,
    ) else {
        return;
    };
    APPEARANCE_REFRESH_CLOCK.with(|clock| {
        if let Some(previous) = clock.borrow_mut().replace(AppearanceRefreshClock {
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
async fn fetch_appearance_view(window: &web_sys::Window) -> Result<AppearanceView, String> {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let value = JsFuture::from(window.fetch_with_str("/api/v1/appearance"))
        .await
        .map_err(|_| "Appearance state request failed".to_owned())?;
    let response = value
        .dyn_into::<web_sys::Response>()
        .map_err(|_| "Appearance state response was invalid".to_owned())?;
    if !response.ok() {
        return Err("Appearance state request was rejected".to_owned());
    }
    let body = response
        .text()
        .map_err(|_| "Appearance state response could not be read".to_owned())?;
    let body = JsFuture::from(body)
        .await
        .map_err(|_| "Appearance state response could not be read".to_owned())?
        .as_string()
        .ok_or_else(|| "Appearance state response was invalid".to_owned())?;
    let appearance = serde_json::from_str::<AppearanceView>(&body)
        .map_err(|_| "Appearance state response was invalid".to_owned())?;
    appearance
        .validate()
        .map_err(|_| "Appearance state response was invalid".to_owned())?;
    Ok(appearance)
}

#[cfg(feature = "hydrate")]
fn request_pet_selection(
    appearance: RwSignal<AppearanceView>,
    selection_error: RwSignal<Option<String>>,
    selection_in_flight: RwSignal<bool>,
    pet_id: PetId,
) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit};

    if selection_in_flight.get_untracked() {
        return;
    }
    let body = match serde_json::to_string(&AppearanceSelectionRequest { pet_id }) {
        Ok(body) => body,
        Err(_) => {
            selection_error.set(Some("Pet selection could not be encoded".to_owned()));
            return;
        }
    };
    let request_init = RequestInit::new();
    request_init.set_method("PUT");
    request_init.set_body(&JsValue::from_str(&body));
    let request = match Request::new_with_str_and_init("/api/v1/appearance/pet", &request_init) {
        Ok(request) => request,
        Err(_) => {
            selection_error.set(Some(
                "Pet selection request could not be created".to_owned(),
            ));
            return;
        }
    };
    if request
        .headers()
        .set("Content-Type", "application/json")
        .is_err()
    {
        selection_error.set(Some(
            "Pet selection request could not be prepared".to_owned(),
        ));
        return;
    }
    let Some(window) = web_sys::window() else {
        selection_error.set(Some("Pet selection is unavailable".to_owned()));
        return;
    };

    selection_error.set(None);
    selection_in_flight.set(true);
    wasm_bindgen_futures::spawn_local(async move {
        let result: Result<AppearanceView, String> = async {
            let value = JsFuture::from(window.fetch_with_request(&request))
                .await
                .map_err(|_| "Pet selection request failed".to_owned())?;
            let response = value
                .dyn_into::<web_sys::Response>()
                .map_err(|_| "Pet selection response was invalid".to_owned())?;
            if !response.ok() {
                return Err("Pet selection was rejected".to_owned());
            }
            let body = response
                .text()
                .map_err(|_| "Pet selection response could not be read".to_owned())?;
            let body = JsFuture::from(body)
                .await
                .map_err(|_| "Pet selection response could not be read".to_owned())?
                .as_string()
                .ok_or_else(|| "Pet selection response was invalid".to_owned())?;
            let next = serde_json::from_str::<AppearanceView>(&body)
                .map_err(|_| "Pet selection response was invalid".to_owned())?;
            next.validate()
                .map_err(|_| "Pet selection response was invalid".to_owned())?;
            Ok(next)
        }
        .await;
        match result {
            Ok(next) => appearance.set(next),
            Err(error) => selection_error.set(Some(error)),
        }
        selection_in_flight.set(false);
    });
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use lili_core::{AppearancePetView, PetId};

    use super::*;

    #[test]
    fn appearance_page_renders_the_focused_pet_surface() {
        let pet_id = PetId::parse("lili").unwrap();
        let html = view! {
            <AppearancePage appearance=AppearanceView {
                pets: vec![AppearancePetView {
                    id: pet_id.clone(),
                    display_name: "Lili".to_owned(),
                    asset_id: "asset-id".to_owned(),
                }],
                selected_pet_id: Some(pet_id),
            }/>
        }
        .to_html();

        assert!(html.contains("data-ssr-marker=\"appearance-ready\""));
        assert!(html.contains("class=\"appearance-window-frame\""));
        assert!(html.contains("data-tauri-drag-region=\"deep\""));
        assert!(html.contains("aria-label=\"Close Appearance window\""));
        assert!(!html.contains("appearance-page-label"));
        assert!(!html.contains("Minimize Appearance window"));
        assert!(!html.contains("appearance-brand-mark"));
        assert!(!html.contains("appearance-desktop-surface"));
        assert!(!html.contains("appearance-scene-badge"));
        assert!(html.contains("class=\"appearance-nav-button\""));
        assert_eq!(html.matches("class=\"appearance-nav-button\"").count(), 1);
        assert!(html.contains(">Pet</span>"));
        assert!(html.contains("id=\"appearance-heading\""));
        assert!(html.contains("/pet-assets/asset-id"));
        assert_eq!(html.matches("appearance-pet-thumb-atlas").count(), 1);
        assert_eq!(html.matches("draggable=\"false\"").count(), 2);
        assert_eq!(html.matches("aria-pressed=").count(), 7);
        assert_eq!(html.matches("data-scene=").count(), 8);
        assert!(!html.contains("Choose a companion"));
        assert!(!html.contains("Pet configuration"));
        assert!(!html.contains("All Pets run locally"));
        assert!(!html.contains("Active pet"));
        assert!(!html.contains(">Notifications</span>"));
        assert!(!html.contains(">Interactions</span>"));
        assert!(!html.contains(">Connection</span>"));
    }

    #[test]
    fn appearance_preview_controller_resets_and_advances_shared_atlas_scheduler() {
        let mut controller = AppearancePreviewController::new(PreviewScene::Idle);
        assert_eq!(controller.scene(), PreviewScene::Idle);
        assert_eq!(controller.frame().row(), 0);
        assert_eq!(controller.frame().column(), 0);

        let first_running_frame = controller.select(PreviewScene::Running);
        assert_eq!(controller.scene(), PreviewScene::Running);
        let expected_running_row =
            AnimationScheduler::new(PreviewScene::Running.spec().animation())
                .current_frame()
                .row();
        assert_eq!(first_running_frame.row(), expected_running_row);

        let next_running_frame = controller.advance(Duration::from_millis(400));
        assert_ne!(next_running_frame, first_running_frame);

        let first_click_frame = controller.select(PreviewScene::Click);
        assert_eq!(controller.scene(), PreviewScene::Click);
        assert_eq!(first_click_frame, controller.frame());
        assert_eq!(
            controller.scene().spec().notification(),
            None,
            "scene previews must not carry a live notification"
        );
    }

    #[test]
    fn appearance_preview_controller_holds_a_frame_when_motion_is_reduced() {
        let mut controller = AppearancePreviewController::new(PreviewScene::Running);
        let running_frame = controller.tick(Duration::from_millis(400), false);
        let held_frame = controller.tick(Duration::from_millis(400), true);

        assert_eq!(held_frame, running_frame);
    }

    #[test]
    fn appearance_animation_tokens_cover_every_standard_animation() {
        let expected = [
            "idle",
            "running-right",
            "running-left",
            "waving",
            "jumping",
            "failed",
            "waiting",
            "running",
            "review",
        ];

        for (spec, expected) in lili_pet::STANDARD_ANIMATIONS.into_iter().zip(expected) {
            assert_eq!(animation_token(spec.state()), expected);
        }
    }

    #[test]
    fn notification_scenes_are_bounded_read_only_preview_cards() {
        for scene in PreviewScene::all() {
            if let Some(notification_kind) = scene.spec().notification() {
                let notification = preview_notification(*scene)
                    .expect("notification scenes must provide preview data");
                assert_eq!(notification.kind, notification_kind);
                assert!(!notification.summary.is_empty());
            }
        }
        assert!(preview_notification(PreviewScene::Idle).is_none());
    }

    #[test]
    fn appearance_notification_preview_reuses_the_notification_card() {
        let notification = preview_notification(PreviewScene::Review).unwrap();
        let html = view! {
            <NotificationPreviewCard
                notification
                wall_clock=RwSignal::new(0_u64)
                reduced_motion=RwSignal::new(false)
            />
        }
        .to_html();

        assert!(html.contains("notification-card"));
        assert!(html.contains("notification-card-preview"));
        assert!(html.contains("notification-card-body"));
        assert!(html.contains("notification-summary"));
        assert!(html.contains("Task completed successfully."));
        assert!(html.contains("disabled"));
        assert!(!html.contains("activateNativeNotification"));
        assert!(!html.contains("dismissNativeNotification"));
    }

    #[test]
    fn appearance_preview_markup_has_no_live_notification_or_action_hooks() {
        let pet_id = PetId::parse("lili").unwrap();
        let html = view! {
            <AppearancePage appearance=AppearanceView {
                pets: vec![AppearancePetView {
                    id: pet_id.clone(),
                    display_name: "Lili".to_owned(),
                    asset_id: "asset-id".to_owned(),
                }],
                selected_pet_id: Some(pet_id),
            }/>
        }
        .to_html();

        for live_hook in [
            "/interactions",
            "/dismiss",
            "activateNative",
            "dismissNative",
            "openNativePetContextMenu",
        ] {
            assert!(
                !html.contains(live_hook),
                "unexpected live hook {live_hook}"
            );
        }
    }

    #[test]
    fn appearance_markup_exposes_keyboard_and_selected_state_semantics() {
        let selected_id = PetId::parse("lili").unwrap();
        let alternate_id = PetId::parse("alternate").unwrap();
        let html = view! {
            <AppearancePage appearance=AppearanceView {
                pets: vec![
                    AppearancePetView {
                        id: selected_id.clone(),
                        display_name: "Lili".to_owned(),
                        asset_id: "asset-id".to_owned(),
                    },
                    AppearancePetView {
                        id: alternate_id,
                        display_name: "Alternate".to_owned(),
                        asset_id: "alternate-asset".to_owned(),
                    },
                ],
                selected_pet_id: Some(selected_id),
            }/>
        }
        .to_html();

        assert!(html.contains("role=\"listbox\""));
        assert_eq!(html.matches("role=\"option\"").count(), 2);
        assert_eq!(html.matches("aria-selected=\"true\"").count(), 1);
        assert_eq!(html.matches("aria-selected=\"false\"").count(), 1);
        assert_eq!(html.matches("aria-pressed=").count(), 7);
        assert!(html.contains("aria-live=\"polite\""));
        assert!(html.contains("data-pet-id=\"alternate\""));
    }

    #[test]
    fn appearance_css_has_wide_and_narrow_layout_guards() {
        let css = include_str!("../../web/lili.css");
        assert!(css.contains(".appearance-window-frame {"));
        assert!(css.contains(".appearance-topbar-leading {"));
        assert!(css.contains(".appearance-selectable,"));
        assert!(css.contains("-webkit-user-select: text;"));
        assert!(css.contains("overflow: visible;"));
        assert!(css.contains("border-radius: 0 0 19px 19px;"));
        assert!(css.contains("min-height: calc(100vh - 96px);"));
        assert!(css.contains("padding: 32px 48px 64px;"));
        assert!(css.contains("@keyframes appearance-idle-preview"));
        assert!(css.contains(".notification-card {"));
        assert!(css.contains(".notification-card-preview"));
        assert!(css.contains("-webkit-user-drag: none;"));
        assert!(css.contains("pointer-events: none;"));
        assert!(!css.contains(".appearance-desktop-surface {"));
        assert!(!css.contains(".appearance-scene-badge {"));
        assert!(css.contains("width: 384px;"));
        assert!(css.contains("height: 572px;"));
        assert!(css.contains("grid-template-columns: 190px minmax(0, 1fr);"));
        assert!(css.contains("grid-template-columns: minmax(0, 1fr) 300px;"));
        assert!(css.contains("@media (max-width: 900px)"));
        assert!(css.contains("@media (max-width: 560px)"));
        assert!(css.contains(".appearance-scene-buttons {\n  display: flex;\n  flex-wrap: wrap;"));
        assert!(css.contains("#lili-appearance.appearance-surface {\n    padding: 0;"));
    }
}
