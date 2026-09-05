#[cfg(any(test, feature = "hydrate"))]
use std::time::Duration;

use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use lili_core::AppearanceSelectionRequest;
#[cfg(feature = "hydrate")]
use lili_core::PetId;
use lili_core::{AppearanceView, PetNotificationKind};
use lili_pet::{AnimationScheduler, FrameDescriptor, PreviewScene};

#[cfg(feature = "hydrate")]
struct AppearanceClock {
    window: web_sys::Window,
    interval_id: i32,
    _callback: wasm_bindgen::closure::Closure<dyn FnMut()>,
}

#[cfg(feature = "hydrate")]
thread_local! {
    static APPEARANCE_CLOCK: std::cell::RefCell<Option<AppearanceClock>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
pub fn AppearancePage(appearance: AppearanceView) -> impl IntoView {
    let appearance = RwSignal::new(appearance);
    let selection_error = RwSignal::new(None::<String>);
    let preview = RwSignal::new(AppearancePreviewController::new(PreviewScene::Idle));
    let preview_frame = RwSignal::new(preview.get_untracked().frame());
    #[cfg(feature = "hydrate")]
    start_appearance_preview_clock(preview, preview_frame);
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
                            request_pet_selection(
                                appearance,
                                selection_error,
                                select_id.clone(),
                            );
                        }
                    >
                        <span class="appearance-pet-thumb">
                            <img src=asset_url.clone() alt="" aria-hidden="true" />
                        </span>
                        <span class="appearance-pet-item-copy">
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
            <header class="appearance-topbar">
                <div class="appearance-brand">
                    <span class="appearance-brand-mark" aria-hidden="true">"✦"</span>
                    <span class="appearance-brand-copy"><strong>"Lili"</strong><span>"Pet Studio"</span></span>
                </div>
                <div class="appearance-top-actions">
                    <span class="appearance-local-status"><span class="appearance-status-dot" aria-hidden="true"></span>"Local only"</span>
                    <span class="appearance-page-label">"Appearance"</span>
                </div>
            </header>

            <div class="appearance-body">
                <nav class="appearance-sidebar" aria-label="Settings sections">
                    <div class="appearance-sidebar-label">"Configure"</div>
                    <button class="appearance-nav-button" type="button" aria-current="page">
                        <span aria-hidden="true">"🐾"</span><span>"Pet"</span>
                    </button>
                    <div class="appearance-sidebar-foot">
                        <strong>"Pet configuration"</strong>
                        <span>"Choose a package and preview each state."</span>
                    </div>
                </nav>

                <section class="appearance-main" aria-labelledby="appearance-heading">
                    <div class="appearance-page-heading">
                        <h1 id="appearance-heading">"Appearance"</h1>
                        <p>"Choose a companion on the right, then switch scenes to inspect how it looks in each desktop state."</p>
                    </div>

                    <div class="appearance-workbench">
                        <section class="appearance-preview-panel" aria-label="Appearance preview">
                            <div class="appearance-panel-heading">
                                <div class="appearance-panel-heading-copy">
                                    <strong>"Live preview"</strong>
                                    <span>"Preview stays local until you select a Pet."</span>
                                </div>
                                <span class="appearance-live-label"><span class="appearance-status-dot" aria-hidden="true"></span>"Live"</span>
                            </div>

                            <div class="appearance-scene-picker">
                                <div class="appearance-scene-heading">
                                    <strong>"Scene"</strong>
                                    <span id="appearance-scene-caption">
                                        {move || scene_caption(preview.get().scene())}
                                    </span>
                                </div>
                                <div class="appearance-scene-buttons" role="group" aria-label="Preview scenes">
                                    {scene_items}
                                </div>
                            </div>

                            <div class="appearance-stage" aria-live="polite">
                                <div class="appearance-desktop-surface" aria-hidden="true">
                                    <div class="appearance-desktop-bar"><span></span><span></span><span></span></div>
                                    <span class="appearance-desktop-copy">"Desktop surface"</span>
                                    <div class="appearance-desktop-lines"></div>
                                </div>
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
                                            <div
                                                class="appearance-notification appearance-preview-notification"
                                                data-kind=move || preview.get().scene().spec().notification().map(notification_token).unwrap_or_default()
                                                aria-label="Read-only notification preview"
                                            >
                                                <span class="appearance-notification-mark" aria-hidden="true">
                                                    {move || notification_glyph(preview.get().scene().spec().notification())}
                                                </span>
                                                <span class="appearance-notification-copy">
                                                    <strong>
                                                        {move || notification_title(preview.get().scene().spec().notification())}
                                                    </strong>
                                                    <span>
                                                        {move || notification_body(preview.get().scene().spec().notification())}
                                                    </span>
                                                </span>
                                                <span class="appearance-notification-action" aria-hidden="true">"↗"</span>
                                            </div>
                                        </Show>
                                    </div>
                                    <div class="appearance-pet" id="appearance-pet">
                                        <img
                                            class="appearance-pet-atlas"
                                            src=selected_asset_url
                                            alt=""
                                            aria-hidden="true"
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
                                <span class="appearance-scene-badge">"Preview only"</span>
                            </div>

                            <div class="appearance-preview-footer">
                                <span id="appearance-preview-footer-copy">
                                    {move || scene_footer(preview.get().scene())}
                                </span>
                                <span>"Single scene"</span>
                            </div>
                        </section>

                        <aside class="appearance-pet-panel" aria-label="Pet list">
                            <h2>"Pet"</h2>
                            <p>"Choose which Pet appears in Lili Pet Studio."</p>
                            <div class="appearance-pet-list-heading">
                                <span>"Installed pets"</span>
                                <span>{move || format!("{} available", appearance.get().pets.len())}</span>
                            </div>
                            <div class="appearance-pet-list" role="listbox" aria-label="Installed pets">
                                {pet_items}
                            </div>
                            <div class="appearance-status-line">
                                <span class="appearance-status-dot" aria-hidden="true"></span>
                                <Show
                                    when=move || selection_error.get().is_some()
                                    fallback=|| view! { <small>"All Pets run locally on your device."</small> }
                                >
                                    <small role="alert" class="appearance-selection-error">
                                        {move || selection_error.get().unwrap_or_else(|| "Pet selection failed".to_owned())}
                                    </small>
                                </Show>
                            </div>
                        </aside>
                    </div>
                </section>
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
                <img src=asset_url alt="" aria-hidden="true" />
            </span>
            <span class="appearance-pet-item-copy">
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

const fn scene_caption(scene: PreviewScene) -> &'static str {
    match scene {
        PreviewScene::Idle => "Idle · no pending notification",
        PreviewScene::Running => "Running · activity animation",
        PreviewScene::Review => "Review · completion notification",
        PreviewScene::Attention => "Attention · attention notification",
        PreviewScene::Failed => "Failed · failure notification",
        PreviewScene::Waiting => "Waiting · attention notification",
        PreviewScene::Click => "Click · waving animation",
    }
}

const fn scene_footer(scene: PreviewScene) -> &'static str {
    match scene {
        PreviewScene::Idle | PreviewScene::Running | PreviewScene::Click => {
            "No notification window is shown in this state."
        }
        PreviewScene::Review => "Read-only completion card is shown above the Pet.",
        PreviewScene::Attention | PreviewScene::Waiting => {
            "Read-only attention card is shown above the Pet."
        }
        PreviewScene::Failed => "Read-only failure card is shown above the Pet.",
    }
}

const fn scene_placement(_scene: PreviewScene) -> &'static str {
    "above"
}

const fn notification_token(notification: PetNotificationKind) -> &'static str {
    notification.as_str()
}

const fn notification_glyph(notification: Option<PetNotificationKind>) -> &'static str {
    match notification {
        Some(PetNotificationKind::Attention) => "!",
        Some(PetNotificationKind::Completion) => "✓",
        Some(PetNotificationKind::Failure) => "×",
        None => "",
    }
}

const fn notification_title(notification: Option<PetNotificationKind>) -> &'static str {
    match notification {
        Some(PetNotificationKind::Attention) => "Needs attention",
        Some(PetNotificationKind::Completion) => "Review ready",
        Some(PetNotificationKind::Failure) => "Run failed",
        None => "No notification",
    }
}

const fn notification_body(notification: Option<PetNotificationKind>) -> &'static str {
    match notification {
        Some(PetNotificationKind::Attention) => "A read-only attention card for this preview.",
        Some(PetNotificationKind::Completion) => "A read-only completion card for this preview.",
        Some(PetNotificationKind::Failure) => "A read-only failure card for this preview.",
        None => "No notification is shown in this preview.",
    }
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
fn start_appearance_preview_clock(
    preview: RwSignal<AppearancePreviewController>,
    preview_frame: RwSignal<FrameDescriptor>,
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
        let mut frame = preview.get_untracked().frame();
        preview.update(|preview| frame = preview.advance(Duration::from_millis(elapsed_ms)));
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
fn request_pet_selection(
    appearance: RwSignal<AppearanceView>,
    selection_error: RwSignal<Option<String>>,
    pet_id: PetId,
) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::JsValue;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit};

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
    wasm_bindgen_futures::spawn_local(async move {
        let response = match JsFuture::from(window.fetch_with_request(&request)).await {
            Ok(value) => match value.dyn_into::<web_sys::Response>() {
                Ok(response) => response,
                Err(_) => {
                    selection_error.set(Some("Pet selection response was invalid".to_owned()));
                    return;
                }
            },
            Err(_) => {
                selection_error.set(Some("Pet selection request failed".to_owned()));
                return;
            }
        };
        if !response.ok() {
            selection_error.set(Some("Pet selection was rejected".to_owned()));
            return;
        }
        let body = match response.text() {
            Ok(body) => match JsFuture::from(body).await {
                Ok(value) => value.as_string().unwrap_or_default(),
                Err(_) => {
                    selection_error
                        .set(Some("Pet selection response could not be read".to_owned()));
                    return;
                }
            },
            Err(_) => {
                selection_error.set(Some("Pet selection response could not be read".to_owned()));
                return;
            }
        };
        let next = match serde_json::from_str::<AppearanceView>(&body) {
            Ok(next) if next.validate().is_ok() => next,
            _ => {
                selection_error.set(Some("Pet selection response was invalid".to_owned()));
                return;
            }
        };
        appearance.set(next);
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
        assert!(html.contains("class=\"appearance-nav-button\""));
        assert_eq!(html.matches("class=\"appearance-nav-button\"").count(), 1);
        assert!(html.contains(">Pet</span>"));
        assert!(html.contains("id=\"appearance-heading\""));
        assert!(html.contains("/pet-assets/asset-id"));
        assert_eq!(html.matches("aria-pressed=").count(), 7);
        assert_eq!(html.matches("data-scene=").count(), 8);
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
    fn notification_scenes_are_bounded_read_only_preview_cards() {
        for scene in PreviewScene::all() {
            let notification = scene.spec().notification();
            if notification.is_some() {
                assert!(!notification_body(notification).is_empty());
                assert!(!notification_title(notification).is_empty());
            }
        }
        assert_eq!(notification_token(PetNotificationKind::Failure), "failure");
    }
}
