#![no_main]

use libfuzzer_sys::fuzz_target;
use lili_actions::{
    ActionLoadContext, MAX_ACTION_CONFIG_BYTES, MAX_ACTION_ENTRIES, load_actions_str,
};

fuzz_target!(|payload: &[u8]| {
    let Ok(source) = std::str::from_utf8(payload) else {
        return;
    };
    let context = ActionLoadContext::new("/", "/", Vec::new());
    let loaded = load_actions_str(source, &context);
    let effective = loaded.effective();
    assert!(effective.actions.len() <= MAX_ACTION_ENTRIES);
    assert!(effective.diagnostics.len() <= MAX_ACTION_ENTRIES);
    if payload.len() > MAX_ACTION_CONFIG_BYTES {
        assert!(loaded.enabled().is_empty());
        assert!(effective.actions.is_empty());
    }
});
