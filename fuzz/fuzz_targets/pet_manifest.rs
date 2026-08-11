#![no_main]

use libfuzzer_sys::fuzz_target;
use lili_pet::{MAX_PET_MANIFEST_BYTES, parse_pet_manifest};

fuzz_target!(|payload: &[u8]| {
    let parsed = parse_pet_manifest(payload);
    if payload.len() > MAX_PET_MANIFEST_BYTES {
        assert!(parsed.is_err());
        return;
    }
    if let Ok(manifest) = parsed {
        assert!(manifest.id().len() <= 128);
        assert!(manifest.display_name().len() <= 128);
        assert!(manifest.description().len() <= 512);
        assert!(serde_json::to_vec(&manifest).unwrap().len() <= MAX_PET_MANIFEST_BYTES);
    }
});
