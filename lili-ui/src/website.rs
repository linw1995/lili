use leptos::prelude::*;

use crate::appearance::PetShowcase;

#[component]
pub(crate) fn Website(asset_base: String) -> impl IntoView {
    let logo_url = format!("{asset_base}assets/logo.png");
    let pet_url = format!("{asset_base}assets/spritesheet.webp");
    view! {
        <div id="lili-website">
            <a class="skip-link" href="#demo">"Skip to demo"</a>
            <header class="nav wrap">
                <a class="brand" href="#" aria-label="Lili home">
                    <img src=logo_url alt="" width="32" height="32"/>"lili"
                </a>
                <a href="https://github.com/linw1995/lili">"GitHub ↗"</a>
            </header>
            <main>
                <section class="hero wrap" aria-labelledby="hero-title">
                    <div class="intro">
                        <h1 id="hero-title">"A desktop pet."<br/><em>"Make it yours."</em></h1>
                        <p class="lead">"Inspired by the pet experience in the ChatGPT desktop app. Built to be extended."</p>
                        <dl class="extensions">
                            <div>
                                <dt><a href="https://github.com/linw1995/lili/blob/main/docs/configuration.md#3-configure-pet-packages">"Custom pets ↗"</a></dt>
                                <dd>"Use your own compatible Pet v2 packages."</dd>
                            </div>
                            <div>
                                <dt><a href="https://github.com/linw1995/lili/blob/main/docs/marketplace.md">"Session integration ↗"</a></dt>
                                <dd>"Connect supported Codex events to pet states and notifications."</dd>
                            </div>
                            <div>
                                <dt><a href="https://github.com/linw1995/lili/blob/main/docs/configuration.md#5-configure-interaction-actions">"Local actions ↗"</a></dt>
                                <dd>"Run your own programs when you interact with the pet or a notification."</dd>
                            </div>
                        </dl>
                        <div class="hero-actions">
                            <a class="button" href="https://github.com/linw1995/lili/releases">"Download Lili ↗"</a>
                            <a href="https://github.com/linw1995/lili/blob/main/docs/configuration.md">"Setup guide"</a>
                        </div>
                        <p class="fine">"Local · Open source · "<a href="https://github.com/linw1995/lili/blob/main/LICENSE">"Apache-2.0"</a></p>
                    </div>
                    <div class="demo" id="demo">
                        <PetShowcase asset_url=pet_url/>
                    </div>
                </section>
            </main>
            <footer class="wrap">
                <span>"Named after a real cat."</span>
                <nav aria-label="Project resources">
                    <a href="https://github.com/linw1995/lili/blob/main/docs/build.md">"Build from source"</a>
                    <a href="https://github.com/linw1995/lili/blob/main/docs/security-and-operations.md">"Security"</a>
                    <a href="https://github.com/linw1995/lili/blob/main/docs/privacy-policy.md">"Privacy"</a>
                    <a href="https://github.com/linw1995/lili/blob/main/docs/support.md">"Support"</a>
                </nav>
            </footer>
        </div>
    }
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::*;

    #[test]
    fn website_composes_the_shared_preview_in_one_document() {
        let html = view! { <Website asset_base="assets/release/".to_owned()/> }.to_html();
        assert!(html.contains("id=\"lili-website\""));
        assert!(html.contains("id=\"hero-title\""));
        assert_eq!(html.matches("<main>").count(), 1);
        assert!(html.contains("class=\"pet-showcase\""));
        assert!(html.contains("notification-card-preview"));
        assert!(html.contains("assets/release/assets/logo.png"));
        assert!(html.contains("assets/release/assets/spritesheet.webp"));
        assert!(!html.contains("<iframe"));
        assert!(!html.contains("lili-appearance"));
    }
}
