use leptos::prelude::*;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export async function startupCommand(command) {
  const invoke = window.__TAURI_INTERNALS__?.invoke;
  const label = window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label;
  if (!invoke || label !== 'appearance') {
    throw new Error('Open the desktop Settings window to change this option.');
  }
  if (command !== 'is_enabled') {
    await invoke(`plugin:autostart|${command}`);
  }
  return await invoke('plugin:autostart|is_enabled');
}
"#)]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(catch, js_name = startupCommand)]
    async fn startup_command(command: &str)
    -> Result<wasm_bindgen::JsValue, wasm_bindgen::JsValue>;
}

#[cfg(feature = "hydrate")]
fn request_startup(
    command: &'static str,
    enabled: RwSignal<Option<bool>>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<String>>,
) {
    if busy.get_untracked() {
        return;
    }
    busy.set(true);
    error.set(None);
    wasm_bindgen_futures::spawn_local(async move {
        match startup_command(command).await {
            Ok(value) => match value.as_bool() {
                Some(value) => enabled.set(Some(value)),
                None => {
                    enabled.set(None);
                    error.set(Some("Unable to read the startup setting.".to_owned()));
                }
            },
            Err(_) => {
                // A failed write may have changed the OS entry; do not display stale state.
                enabled.set(None);
                error.set(Some(
                    "Unable to update or read the startup setting. Try again.".to_owned(),
                ));
            }
        }
        busy.set(false);
    });
}

#[component]
pub(crate) fn StartupSettings() -> impl IntoView {
    let enabled = RwSignal::new(None::<bool>);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let native = RwSignal::new(false);
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        let is_native = super::appearance::appearance_window_is_native();
        native.set(is_native);
        if is_native {
            request_startup("is_enabled", enabled, busy, error);
        }
    });
    #[cfg(feature = "hydrate")]
    {
        let listener = window_event_listener(leptos::ev::focus, move |_| {
            if native.get_untracked() {
                request_startup("is_enabled", enabled, busy, error);
            }
        });
        on_cleanup(move || listener.remove());
        let shown_listener = window_event_listener(
            leptos::ev::Custom::<web_sys::Event>::new("lili-appearance-shown"),
            move |_| {
                if native.get_untracked() {
                    request_startup("is_enabled", enabled, busy, error);
                }
            },
        );
        on_cleanup(move || shown_listener.remove());
    }

    view! {
        <section class="appearance-startup" aria-labelledby="startup-heading">
            <div class="appearance-startup-row">
                <div class="appearance-selectable">
                    <h2 id="startup-heading">"Startup"</h2>
                    <p id="startup-description">"Launch Lili automatically when you sign in."</p>
                </div>
                <button
                    class="appearance-startup-button"
                    type="button"
                    role="switch"
                    aria-label="Launch at login"
                    aria-describedby="startup-description"
                    aria-checked=move || enabled.get().unwrap_or(false).to_string()
                    disabled=move || !native.get() || busy.get() || enabled.get().is_none()
                    on:click=move |_| {
                        #[cfg(feature = "hydrate")]
                        request_startup(
                            if enabled.get_untracked() == Some(true) { "disable" } else { "enable" },
                            enabled, busy, error,
                        );
                    }
                >
                    {move || if busy.get() { "Loading…" } else {
                        match enabled.get() { Some(true) => "On", Some(false) => "Off", None => "Unavailable" }
                    }}
                </button>
            </div>
            <Show when=move || !native.get()>
                <p class="appearance-selectable">"Available in the desktop Settings window."</p>
            </Show>
            <Show when=move || error.get().is_some()>
                <p role="alert" class="appearance-selectable">{move || error.get().unwrap_or_default()}</p>
                <button
                    class="appearance-startup-button"
                    type="button"
                    disabled=move || busy.get()
                    on:click=move |_| {
                        #[cfg(feature = "hydrate")]
                        request_startup("is_enabled", enabled, busy, error);
                    }
                >"Retry"</button>
            </Show>
        </section>
    }
}
