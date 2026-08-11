use lili_session::{
    ForwardingCredentials, ForwardingProtocolError, ForwardingVerifier, MAX_FORWARDING_FRAME_BYTES,
    ProviderCapabilitiesInputV1, ProviderInputV1, normalize_provider_input,
};

fn event() -> lili_session::NormalizedSessionEvent {
    normalize_provider_input(ProviderInputV1 {
        version: 1,
        provider: Some("codex".to_owned()),
        event_type: Some("turn_completed".to_owned()),
        event_id: Some("event-1".to_owned()),
        session_id: Some("session-1".to_owned()),
        turn_id: Some("turn-1".to_owned()),
        occurred_at_ms: Some(10),
        project: None,
        summary: None,
        capabilities: ProviderCapabilitiesInputV1::default(),
        source_discriminator: None,
    })
    .unwrap()
}

fn payload(frame: &[u8]) -> &[u8] {
    let length = u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize;
    assert_eq!(length, frame.len() - 4);
    &frame[4..]
}

#[test]
fn forged_mac_is_rejected_without_accepting_the_nonce() {
    let credentials = ForwardingCredentials::generate().unwrap();
    let message = credentials.sign(event(), 1_000).unwrap();
    let frame = message.to_frame().unwrap();
    let mut forged: serde_json::Value = serde_json::from_slice(payload(&frame)).unwrap();
    forged["mac"] = serde_json::Value::String("00".repeat(32));
    let forged = serde_json::to_vec(&forged).unwrap();
    let mut verifier = ForwardingVerifier::new(credentials);
    assert_eq!(
        verifier.verify_payload(&forged, 1_000),
        Err(ForwardingProtocolError::InvalidMac)
    );
    assert!(verifier.verify_payload(payload(&frame), 1_000).is_ok());
}

#[test]
fn replayed_nonce_and_oversized_payload_are_rejected() {
    let credentials = ForwardingCredentials::generate().unwrap();
    let message = credentials.sign(event(), 1_000).unwrap();
    let frame = message.to_frame().unwrap();
    let mut verifier = ForwardingVerifier::new(credentials);
    assert!(verifier.verify_payload(payload(&frame), 1_000).is_ok());
    assert_eq!(
        verifier.verify_payload(payload(&frame), 1_001),
        Err(ForwardingProtocolError::ReplayedNonce)
    );
    assert_eq!(
        verifier.verify_payload(&vec![b'x'; MAX_FORWARDING_FRAME_BYTES + 1], 1_001),
        Err(ForwardingProtocolError::TooLarge)
    );
}
