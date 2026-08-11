use leptos::prelude::*;

#[component]
pub fn App(#[prop(optional)] pet_asset_url: String) -> impl IntoView {
    let asset_source = (!pet_asset_url.is_empty()).then_some(pet_asset_url.clone());
    view! {
        <main
            id="lili-app"
            data-ssr-marker="lili-ready"
            data-pet-asset-url=pet_asset_url
        >
            <section class="pet-sprite" aria-label="Lili desktop pet">
                <img class="pet-atlas" src=asset_source alt="" aria-hidden="true"/>
            </section>
        </main>
    }
}

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn hydrate() {
    let pet_asset_url = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("lili-app"))
        .and_then(|element| element.get_attribute("data-pet-asset-url"))
        .unwrap_or_default();
    leptos::mount::hydrate_body(move || {
        view! { <App pet_asset_url=pet_asset_url.clone()/> }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssr_shell_contains_the_approved_asset_url() {
        let html = view! {
            <App pet_asset_url="/api/v1/pet-assets/opaque-id".to_owned()/>
        }
        .to_html();
        assert!(html.contains("class=\"pet-sprite\""));
        assert!(html.contains("class=\"pet-atlas\""));
        assert!(html.contains("/api/v1/pet-assets/opaque-id"));
        assert!(!html.contains("spritesheet.webp"));
    }
}
