use leptos::prelude::*;
use lili_core::PetPresentationState;

#[component]
pub fn App(presentation: PetPresentationState) -> impl IntoView {
    let serialized = serde_json::to_string(&presentation).unwrap_or_else(|_| "{}".to_owned());
    let asset_source = presentation
        .pet_asset_id
        .as_ref()
        .map(|asset_id| format!("/pet-assets/{asset_id}"));
    let lifecycle = presentation.lifecycle.as_str();
    let accessible_label = format!("{}, {lifecycle}", presentation.pet_label);
    view! {
        <main
            id="lili-app"
            data-ssr-marker="lili-ready"
            data-presentation=serialized
            data-revision=presentation.revision
            data-lifecycle=lifecycle
            data-unread-count=presentation.unread_notification_count
        >
            <section
                class="pet-sprite"
                aria-label=accessible_label
                data-tauri-drag-region="deep"
            >
                <img class="pet-atlas" src=asset_source alt="" aria-hidden="true"/>
            </section>
        </main>
    }
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
}
