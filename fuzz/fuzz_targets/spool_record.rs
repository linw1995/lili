#![no_main]

use libfuzzer_sys::fuzz_target;
use lili_session::{MAX_SPOOL_RECORD_BYTES, decode_spool_record};

fuzz_target!(|payload: &[u8]| {
    let parsed = decode_spool_record(payload);
    if payload.len() > MAX_SPOOL_RECORD_BYTES {
        assert!(parsed.is_err());
        return;
    }
    if let Ok((_, event)) = parsed {
        assert!(serde_json::to_vec(&event).unwrap().len() <= MAX_SPOOL_RECORD_BYTES);
    }
});
