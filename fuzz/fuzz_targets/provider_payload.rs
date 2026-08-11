#![no_main]

use libfuzzer_sys::fuzz_target;
use lili_session::{MAX_PROVIDER_PAYLOAD_BYTES, normalize_json};

fuzz_target!(|payload: &[u8]| {
    let parsed = normalize_json(payload);
    if payload.len() > MAX_PROVIDER_PAYLOAD_BYTES {
        assert!(parsed.is_err());
        return;
    }
    if let Ok(event) = parsed {
        assert!(serde_json::to_vec(&event).unwrap().len() <= MAX_PROVIDER_PAYLOAD_BYTES);
    }
});
