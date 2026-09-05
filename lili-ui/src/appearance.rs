use leptos::prelude::*;
#[cfg(feature = "hydrate")]
use lili_core::AppearanceSelectionRequest;
use lili_core::AppearanceView;
#[cfg(feature = "hydrate")]
use lili_core::PetId;
use lili_pet::PreviewScene;

#[component]
pub fn AppearancePage(appearance: AppearanceView) -> impl IntoView {
    let appearance = RwSignal::new(appearance);
    let selection_error = RwSignal::new(None::<String>);
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
        .map(appearance_scene_button)
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
                                    <span id="appearance-scene-caption">"Idle · no pending notification"</span>
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
                                <div class="appearance-scene" id="appearance-scene" data-placement="above">
                                    <div class="appearance-notifications" id="appearance-notifications">
                                        <div class="appearance-preview-empty">"No pending notifications"</div>
                                    </div>
                                    <div class="appearance-pet" id="appearance-pet">
                                        <img
                                            class="appearance-pet-atlas"
                                            src=selected_asset_url
                                            alt=""
                                            aria-hidden="true"
                                        />
                                        <span class="appearance-pet-tag">"Idle"</span>
                                    </div>
                                </div>
                                <span class="appearance-scene-badge">"Preview only"</span>
                            </div>

                            <div class="appearance-preview-footer">
                                <span id="appearance-preview-footer-copy">"No notification window is shown in this state."</span>
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

fn appearance_scene_button(scene: PreviewScene) -> impl IntoView {
    let selected = scene == PreviewScene::Idle;
    view! {
        <button
            class="appearance-scene-button"
            class:appearance-scene-selected=selected
            type="button"
            aria-pressed=selected.to_string()
            data-scene=scene_token(scene)
        >
            <span class="appearance-scene-glyph" aria-hidden="true">{scene_glyph(scene)}</span>
            <span>{scene_label(scene)}</span>
        </button>
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
        assert_eq!(html.matches("data-scene=").count(), 7);
        assert!(!html.contains("Active pet"));
        assert!(!html.contains(">Notifications</span>"));
        assert!(!html.contains(">Interactions</span>"));
        assert!(!html.contains(">Connection</span>"));
    }
}
