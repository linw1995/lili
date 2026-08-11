#![no_main]

use libfuzzer_sys::fuzz_target;
use lili_actions::{MAX_INTERACTION_CONTEXT_BYTES, decode_interaction_context};

fuzz_target!(|payload: &[u8]| {
    let parsed = decode_interaction_context(payload);
    if payload.len() > MAX_INTERACTION_CONTEXT_BYTES {
        assert!(parsed.is_err());
        return;
    }
    if let Ok(context) = parsed {
        assert!(serde_json::to_vec(&context).unwrap().len() <= MAX_INTERACTION_CONTEXT_BYTES);
    }
});
