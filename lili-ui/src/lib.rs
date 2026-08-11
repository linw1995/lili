use leptos::prelude::*;

#[component]
pub fn App() -> impl IntoView {
    view! {
        <main id="lili-app" data-ssr-marker="lili-ready">
            <section class="pet-placeholder" aria-label="Desktop pet preview">
                <span>"Lili"</span>
            </section>
        </main>
    }
}

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn hydrate() {
    leptos::mount::hydrate_body(App);
}
