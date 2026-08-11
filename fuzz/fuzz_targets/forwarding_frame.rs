#![no_main]

use std::sync::LazyLock;

use libfuzzer_sys::fuzz_target;
use lili_session::{ForwardingCredentials, ForwardingVerifier, MAX_FORWARDING_FRAME_BYTES};

static CREDENTIALS: LazyLock<ForwardingCredentials> =
    LazyLock::new(|| ForwardingCredentials::generate().unwrap());

fuzz_target!(|payload: &[u8]| {
    let mut verifier = ForwardingVerifier::new(CREDENTIALS.clone());
    let parsed = verifier.verify_payload(payload, 1_000);
    if payload.len() > MAX_FORWARDING_FRAME_BYTES {
        assert!(parsed.is_err());
    }
    if let Ok(message) = parsed {
        assert!(serde_json::to_vec(message.event()).unwrap().len() <= MAX_FORWARDING_FRAME_BYTES);
    }
});
