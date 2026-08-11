use leptos::prelude::*;
use lili_core::PetPresentationState;

#[component]
pub fn App(presentation: PetPresentationState) -> impl IntoView {
    let presentation = RwSignal::new(presentation);
    #[cfg(feature = "hydrate")]
    connect_presentation_stream(presentation);
    view! {
        <main
            id="lili-app"
            data-ssr-marker="lili-ready"
            data-presentation=move || serde_json::to_string(&presentation.get()).unwrap_or_default()
            data-revision=move || presentation.get().revision
            data-lifecycle=move || presentation.get().lifecycle.as_str()
            data-unread-count=move || presentation.get().unread_notification_count
        >
            <section
                class="pet-sprite"
                aria-label=move || {
                    let state = presentation.get();
                    format!("{}, {}", state.pet_label, state.lifecycle.as_str())
                }
                data-tauri-drag-region="deep"
            >
                <img
                    class="pet-atlas"
                    src=move || presentation.get().pet_asset_id.map(|asset_id| format!("/pet-assets/{asset_id}"))
                    alt=""
                    aria-hidden="true"
                />
            </section>
        </main>
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
fn connect_presentation_stream(presentation: RwSignal<PetPresentationState>) {
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
struct PresentationStream {
    source: web_sys::EventSource,
    _callback: wasm_bindgen::closure::Closure<dyn FnMut(web_sys::MessageEvent)>,
}

#[cfg(feature = "hydrate")]
thread_local! {
    static PRESENTATION_STREAM: std::cell::RefCell<Option<PresentationStream>> =
        const { std::cell::RefCell::new(None) };
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
}
