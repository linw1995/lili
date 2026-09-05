use leptos::prelude::*;
use lili_core::AppearanceView;
use lili_pet::PreviewScene;

#[component]
pub fn AppearancePage(appearance: AppearanceView) -> impl IntoView {
    let selected_pet_id = appearance.selected_pet_id.clone();
    let selected_asset_id = selected_pet_id
        .as_ref()
        .and_then(|selected| appearance.pets.iter().find(|pet| &pet.id == selected))
        .map(|pet| pet.asset_id.clone());
    let serialized_appearance = serde_json::to_string(&appearance).unwrap_or_default();
    let pet_items = appearance
        .pets
        .iter()
        .cloned()
        .map(|pet| {
            let selected = selected_pet_id.as_ref() == Some(&pet.id);
            appearance_pet_item(pet, selected)
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
                                        {selected_asset_id.map(|asset_id| view! {
                                            <img
                                                class="appearance-pet-atlas"
                                                src=format!("/pet-assets/{asset_id}")
                                                alt=""
                                                aria-hidden="true"
                                            />
                                        })}
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
                                <span>{appearance.pets.len()} " available"</span>
                            </div>
                            <div class="appearance-pet-list" role="listbox" aria-label="Installed pets">
                                {pet_items}
                            </div>
                            <div class="appearance-status-line">
                                <span class="appearance-status-dot" aria-hidden="true"></span>
                                <small>"All Pets run locally on your device."</small>
                            </div>
                        </aside>
                    </div>
                </section>
            </div>
        </main>
    }
}

fn appearance_pet_item(pet: lili_core::AppearancePetView, selected: bool) -> impl IntoView {
    let pet_id = pet.id.as_str().to_owned();
    let asset_id = pet.asset_id;
    view! {
        <button
            class="appearance-pet-item"
            class:appearance-pet-selected=selected
            type="button"
            role="option"
            aria-selected=selected.to_string()
            data-pet-id=pet_id
        >
            <span class="appearance-pet-thumb">
                <img src=format!("/pet-assets/{asset_id}") alt="" aria-hidden="true" />
            </span>
            <span class="appearance-pet-item-copy">
                <strong>{pet.display_name}</strong>
                <span>{if selected { "Selected" } else { "Installed package" }}</span>
            </span>
            <span class="appearance-pet-item-check" aria-hidden="true">{if selected { "✓" } else { "" }}</span>
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
