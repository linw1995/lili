use std::{
    collections::{BTreeMap, VecDeque},
    fmt,
    path::{Path, PathBuf},
};

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::Sha256;

use crate::NormalizedSessionEvent;

pub const FORWARDING_PROTOCOL_VERSION: u16 = 1;
pub const MAX_FORWARDING_FRAME_BYTES: usize = 64 * 1024;
pub const DEFAULT_REPLAY_WINDOW_MS: u64 = 60_000;
pub const DEFAULT_NONCE_CAPACITY: usize = 4096;

const INSTANCE_ID_BYTES: usize = 16;
const NONCE_BYTES: usize = 16;
const SECRET_BYTES: usize = 32;
const MAC_BYTES: usize = 32;
const MAX_ENDPOINT_TEXT_BYTES: usize = 1024;
const SIGNATURE_DOMAIN: &[u8] = b"lili-forwarding-v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlatformEndpoint {
    UnixSocket { path: PathBuf },
    WindowsNamedPipe { name: String },
}

impl PlatformEndpoint {
    pub fn unix_socket(path: impl Into<PathBuf>) -> Result<Self, ForwardingProtocolError> {
        let endpoint = Self::UnixSocket { path: path.into() };
        endpoint.validate()?;
        Ok(endpoint)
    }

    pub fn windows_named_pipe(name: impl Into<String>) -> Result<Self, ForwardingProtocolError> {
        let endpoint = Self::WindowsNamedPipe { name: name.into() };
        endpoint.validate()?;
        Ok(endpoint)
    }

    pub fn validate(&self) -> Result<(), ForwardingProtocolError> {
        match self {
            Self::UnixSocket { path } => validate_endpoint_text(path.as_os_str().to_string_lossy()),
            Self::WindowsNamedPipe { name } => {
                validate_endpoint_text(name.as_str().into())?;
                if !name.starts_with(r"\\.\pipe\lili-") {
                    return Err(ForwardingProtocolError::InvalidEndpoint);
                }
                Ok(())
            }
        }
    }

    pub fn unix_path(&self) -> Option<&Path> {
        match self {
            Self::UnixSocket { path } => Some(path),
            Self::WindowsNamedPipe { .. } => None,
        }
    }

    pub fn named_pipe(&self) -> Option<&str> {
        match self {
            Self::UnixSocket { .. } => None,
            Self::WindowsNamedPipe { name } => Some(name),
        }
    }
}

fn validate_endpoint_text(value: std::borrow::Cow<'_, str>) -> Result<(), ForwardingProtocolError> {
    if value.is_empty()
        || value.len() > MAX_ENDPOINT_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(ForwardingProtocolError::InvalidEndpoint);
    }
    Ok(())
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardingCredentialRecord {
    version: u16,
    instance_id: String,
    secret: String,
    endpoint: PlatformEndpoint,
}

impl fmt::Debug for ForwardingCredentialRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ForwardingCredentialRecord")
            .field("version", &self.version)
            .field("instance_id", &self.instance_id)
            .field("secret", &"[redacted]")
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

impl ForwardingCredentialRecord {
    pub fn new(credentials: &ForwardingCredentials, endpoint: PlatformEndpoint) -> Self {
        Self {
            version: FORWARDING_PROTOCOL_VERSION,
            instance_id: credentials.instance_id.clone(),
            secret: encode_hex(&credentials.secret),
            endpoint,
        }
    }

    pub fn credentials(&self) -> Result<ForwardingCredentials, ForwardingProtocolError> {
        if self.version != FORWARDING_PROTOCOL_VERSION {
            return Err(ForwardingProtocolError::UnsupportedVersion(self.version));
        }
        validate_hex(&self.instance_id, INSTANCE_ID_BYTES)
            .ok_or(ForwardingProtocolError::InvalidInstanceId)?;
        let secret = decode_hex_array::<SECRET_BYTES>(&self.secret)
            .ok_or(ForwardingProtocolError::InvalidCredential)?;
        self.endpoint.validate()?;
        Ok(ForwardingCredentials {
            instance_id: self.instance_id.clone(),
            secret,
        })
    }

    pub fn endpoint(&self) -> &PlatformEndpoint {
        &self.endpoint
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub const fn version(&self) -> u16 {
        self.version
    }
}

#[derive(Clone)]
pub struct ForwardingCredentials {
    instance_id: String,
    secret: [u8; SECRET_BYTES],
}

impl fmt::Debug for ForwardingCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ForwardingCredentials")
            .field("instance_id", &self.instance_id)
            .field("secret", &"[redacted]")
            .finish()
    }
}

impl ForwardingCredentials {
    pub fn generate() -> Result<Self, ForwardingProtocolError> {
        let mut instance_id = [0_u8; INSTANCE_ID_BYTES];
        let mut secret = [0_u8; SECRET_BYTES];
        getrandom::fill(&mut instance_id).map_err(|_| ForwardingProtocolError::Randomness)?;
        getrandom::fill(&mut secret).map_err(|_| ForwardingProtocolError::Randomness)?;
        Ok(Self {
            instance_id: encode_hex(&instance_id),
            secret,
        })
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn sign(
        &self,
        event: NormalizedSessionEvent,
        sent_at_ms: u64,
    ) -> Result<ForwardingMessage, ForwardingProtocolError> {
        let mut nonce = [0_u8; NONCE_BYTES];
        getrandom::fill(&mut nonce).map_err(|_| ForwardingProtocolError::Randomness)?;
        self.sign_with_nonce(event, sent_at_ms, encode_hex(&nonce))
    }

    fn sign_with_nonce(
        &self,
        event: NormalizedSessionEvent,
        sent_at_ms: u64,
        nonce: String,
    ) -> Result<ForwardingMessage, ForwardingProtocolError> {
        validate_hex(&nonce, NONCE_BYTES).ok_or(ForwardingProtocolError::InvalidNonce)?;
        let unsigned = UnsignedForwardingMessage {
            version: FORWARDING_PROTOCOL_VERSION,
            instance_id: self.instance_id.clone(),
            nonce,
            sent_at_ms,
            event,
        };
        let mac = compute_mac(&self.secret, &unsigned)?;
        Ok(ForwardingMessage {
            version: unsigned.version,
            instance_id: unsigned.instance_id,
            nonce: unsigned.nonce,
            sent_at_ms: unsigned.sent_at_ms,
            event: unsigned.event,
            mac: encode_hex(&mac),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardingMessage {
    version: u16,
    instance_id: String,
    nonce: String,
    sent_at_ms: u64,
    event: NormalizedSessionEvent,
    mac: String,
}

impl ForwardingMessage {
    pub fn to_frame(&self) -> Result<Vec<u8>, ForwardingProtocolError> {
        encode_frame(self)
    }

    pub fn event(&self) -> &NormalizedSessionEvent {
        &self.event
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardingAckDisposition {
    Accepted,
    Duplicate,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardingAck {
    version: u16,
    instance_id: String,
    nonce: String,
    disposition: ForwardingAckDisposition,
}

impl ForwardingAck {
    pub fn to_frame(&self) -> Result<Vec<u8>, ForwardingProtocolError> {
        encode_frame(self)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, ForwardingProtocolError> {
        let acknowledgement: Self = decode_payload(payload)?;
        if acknowledgement.version != FORWARDING_PROTOCOL_VERSION {
            return Err(ForwardingProtocolError::UnsupportedVersion(
                acknowledgement.version,
            ));
        }
        validate_hex(&acknowledgement.instance_id, INSTANCE_ID_BYTES)
            .ok_or(ForwardingProtocolError::InvalidInstanceId)?;
        validate_hex(&acknowledgement.nonce, NONCE_BYTES)
            .ok_or(ForwardingProtocolError::InvalidNonce)?;
        Ok(acknowledgement)
    }

    pub const fn disposition(&self) -> ForwardingAckDisposition {
        self.disposition
    }

    pub fn validate_for(&self, message: &ForwardingMessage) -> Result<(), ForwardingProtocolError> {
        if self.version != FORWARDING_PROTOCOL_VERSION {
            return Err(ForwardingProtocolError::UnsupportedVersion(self.version));
        }
        if self.instance_id != message.instance_id || self.nonce != message.nonce {
            return Err(ForwardingProtocolError::MismatchedAcknowledgement);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedForwardingMessage {
    event: NormalizedSessionEvent,
    instance_id: String,
    nonce: String,
}

impl VerifiedForwardingMessage {
    pub fn event(&self) -> &NormalizedSessionEvent {
        &self.event
    }

    pub fn into_event(self) -> NormalizedSessionEvent {
        self.event
    }

    pub fn acknowledgement(&self, disposition: ForwardingAckDisposition) -> ForwardingAck {
        ForwardingAck {
            version: FORWARDING_PROTOCOL_VERSION,
            instance_id: self.instance_id.clone(),
            nonce: self.nonce.clone(),
            disposition,
        }
    }
}

pub struct ForwardingVerifier {
    credentials: ForwardingCredentials,
    accepted_nonces: VecDeque<(String, u64)>,
    accepted_nonce_set: BTreeMap<String, u64>,
    replay_window_ms: u64,
    nonce_capacity: usize,
}

impl ForwardingVerifier {
    pub fn new(credentials: ForwardingCredentials) -> Self {
        Self::with_limits(
            credentials,
            DEFAULT_REPLAY_WINDOW_MS,
            DEFAULT_NONCE_CAPACITY,
        )
    }

    pub fn with_limits(
        credentials: ForwardingCredentials,
        replay_window_ms: u64,
        nonce_capacity: usize,
    ) -> Self {
        Self {
            credentials,
            accepted_nonces: VecDeque::new(),
            accepted_nonce_set: BTreeMap::new(),
            replay_window_ms,
            nonce_capacity: nonce_capacity.max(1),
        }
    }

    pub fn verify_payload(
        &mut self,
        payload: &[u8],
        now_ms: u64,
    ) -> Result<VerifiedForwardingMessage, ForwardingProtocolError> {
        let message: ForwardingMessage = decode_payload(payload)?;
        if message.version != FORWARDING_PROTOCOL_VERSION {
            return Err(ForwardingProtocolError::UnsupportedVersion(message.version));
        }
        if message.instance_id != self.credentials.instance_id {
            return Err(ForwardingProtocolError::WrongInstance);
        }
        validate_hex(&message.instance_id, INSTANCE_ID_BYTES)
            .ok_or(ForwardingProtocolError::InvalidInstanceId)?;
        validate_hex(&message.nonce, NONCE_BYTES).ok_or(ForwardingProtocolError::InvalidNonce)?;
        if now_ms.abs_diff(message.sent_at_ms) > self.replay_window_ms {
            return Err(ForwardingProtocolError::Expired);
        }
        let supplied_mac = decode_hex_array::<MAC_BYTES>(&message.mac)
            .ok_or(ForwardingProtocolError::InvalidMac)?;
        let unsigned = UnsignedForwardingMessage {
            version: message.version,
            instance_id: message.instance_id.clone(),
            nonce: message.nonce.clone(),
            sent_at_ms: message.sent_at_ms,
            event: message.event.clone(),
        };
        let signing_bytes = signing_bytes(&unsigned)?;
        Hmac::<Sha256>::new_from_slice(&self.credentials.secret)
            .expect("HMAC accepts a key of any size")
            .chain_update(signing_bytes)
            .verify_slice(&supplied_mac)
            .map_err(|_| ForwardingProtocolError::InvalidMac)?;

        self.prune_nonces(now_ms);
        if self.accepted_nonce_set.contains_key(&message.nonce) {
            return Err(ForwardingProtocolError::ReplayedNonce);
        }
        while self.accepted_nonces.len() >= self.nonce_capacity {
            if let Some((nonce, _)) = self.accepted_nonces.pop_front() {
                self.accepted_nonce_set.remove(&nonce);
            }
        }
        self.accepted_nonces
            .push_back((message.nonce.clone(), now_ms));
        self.accepted_nonce_set
            .insert(message.nonce.clone(), now_ms);

        Ok(VerifiedForwardingMessage {
            event: message.event,
            instance_id: message.instance_id,
            nonce: message.nonce,
        })
    }

    fn prune_nonces(&mut self, now_ms: u64) {
        while self
            .accepted_nonces
            .front()
            .is_some_and(|(_, accepted_at)| {
                now_ms.saturating_sub(*accepted_at) > self.replay_window_ms
            })
        {
            if let Some((nonce, _)) = self.accepted_nonces.pop_front() {
                self.accepted_nonce_set.remove(&nonce);
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnsignedForwardingMessage {
    version: u16,
    instance_id: String,
    nonce: String,
    sent_at_ms: u64,
    event: NormalizedSessionEvent,
}

fn compute_mac(
    secret: &[u8; SECRET_BYTES],
    message: &UnsignedForwardingMessage,
) -> Result<[u8; MAC_BYTES], ForwardingProtocolError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts a key of any size");
    mac.update(&signing_bytes(message)?);
    Ok(mac.finalize().into_bytes().into())
}

fn signing_bytes(message: &UnsignedForwardingMessage) -> Result<Vec<u8>, ForwardingProtocolError> {
    let payload = serde_json::to_vec(message).map_err(|_| ForwardingProtocolError::Malformed)?;
    let mut bytes = Vec::with_capacity(SIGNATURE_DOMAIN.len() + payload.len() + 16);
    bytes.extend_from_slice(&(SIGNATURE_DOMAIN.len() as u64).to_be_bytes());
    bytes.extend_from_slice(SIGNATURE_DOMAIN);
    bytes.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, ForwardingProtocolError> {
    let payload = serde_json::to_vec(value).map_err(|_| ForwardingProtocolError::Malformed)?;
    if payload.len() > MAX_FORWARDING_FRAME_BYTES {
        return Err(ForwardingProtocolError::TooLarge);
    }
    let length = u32::try_from(payload.len()).map_err(|_| ForwardingProtocolError::TooLarge)?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn decode_payload<T: DeserializeOwned>(payload: &[u8]) -> Result<T, ForwardingProtocolError> {
    if payload.len() > MAX_FORWARDING_FRAME_BYTES {
        return Err(ForwardingProtocolError::TooLarge);
    }
    serde_json::from_slice(payload).map_err(|_| ForwardingProtocolError::Malformed)
}

fn validate_hex(value: &str, bytes: usize) -> Option<()> {
    (value.len() == bytes * 2 && value.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(())
}

fn encode_hex(bytes: &[u8]) -> String {
    use fmt::Write as _;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn decode_hex_array<const N: usize>(value: &str) -> Option<[u8; N]> {
    validate_hex(value, N)?;
    let mut decoded = [0_u8; N];
    for (index, byte) in decoded.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(decoded)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForwardingProtocolError {
    TooLarge,
    Malformed,
    UnsupportedVersion(u16),
    InvalidEndpoint,
    InvalidInstanceId,
    InvalidCredential,
    InvalidNonce,
    InvalidMac,
    WrongInstance,
    Expired,
    ReplayedNonce,
    MismatchedAcknowledgement,
    Randomness,
}

impl fmt::Display for ForwardingProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::TooLarge => "forwarding frame exceeds the size limit",
            Self::Malformed => "forwarding frame is malformed",
            Self::UnsupportedVersion(_) => "forwarding protocol version is unsupported",
            Self::InvalidEndpoint => "forwarding endpoint is invalid",
            Self::InvalidInstanceId => "forwarding instance identity is invalid",
            Self::InvalidCredential => "forwarding credential is invalid",
            Self::InvalidNonce => "forwarding nonce is invalid",
            Self::InvalidMac => "forwarding authentication failed",
            Self::WrongInstance => "forwarding credential belongs to another instance",
            Self::Expired => "forwarding message is outside the replay window",
            Self::ReplayedNonce => "forwarding nonce was already accepted",
            Self::MismatchedAcknowledgement => {
                "forwarding acknowledgement does not match the message"
            }
            Self::Randomness => "secure randomness is unavailable",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ForwardingProtocolError {}

#[cfg(test)]
mod tests {
    use crate::{ProviderCapabilitiesInputV1, ProviderInputV1, normalize_provider_input};

    use super::*;

    fn event() -> NormalizedSessionEvent {
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
    fn signed_message_and_acknowledgement_round_trip() {
        let credentials = ForwardingCredentials::generate().unwrap();
        let message = credentials.sign(event(), 1_000).unwrap();
        let frame = message.to_frame().unwrap();
        let mut verifier = ForwardingVerifier::new(credentials);
        let verified = verifier.verify_payload(payload(&frame), 1_001).unwrap();
        assert_eq!(verified.event(), message.event());

        let ack = verified.acknowledgement(ForwardingAckDisposition::Accepted);
        let ack_frame = ack.to_frame().unwrap();
        let restored = ForwardingAck::from_payload(payload(&ack_frame)).unwrap();
        assert_eq!(restored, ack);
        restored.validate_for(&message).unwrap();
    }

    #[test]
    fn credentials_rotate_and_debug_output_redacts_the_secret() {
        let endpoint = PlatformEndpoint::unix_socket("/tmp/lili.sock").unwrap();
        let first = ForwardingCredentials::generate().unwrap();
        let second = ForwardingCredentials::generate().unwrap();
        let record = ForwardingCredentialRecord::new(&first, endpoint);
        assert!(!format!("{record:?}").contains(&record.secret));

        let message = first.sign(event(), 1_000).unwrap();
        let mut verifier = ForwardingVerifier::new(second);
        assert_eq!(
            verifier.verify_payload(payload(&message.to_frame().unwrap()), 1_000),
            Err(ForwardingProtocolError::WrongInstance)
        );
    }

    #[test]
    fn nonce_is_accepted_only_once_within_the_window() {
        let credentials = ForwardingCredentials::generate().unwrap();
        let message = credentials
            .sign_with_nonce(event(), 1_000, "ab".repeat(NONCE_BYTES))
            .unwrap();
        let frame = message.to_frame().unwrap();
        let mut verifier = ForwardingVerifier::new(credentials);
        assert!(verifier.verify_payload(payload(&frame), 1_001).is_ok());
        assert_eq!(
            verifier.verify_payload(payload(&frame), 1_002),
            Err(ForwardingProtocolError::ReplayedNonce)
        );
    }

    #[test]
    fn stale_and_future_messages_are_rejected() {
        let credentials = ForwardingCredentials::generate().unwrap();
        let past = credentials.sign(event(), 1_000).unwrap();
        let future = credentials.sign(event(), 100_000).unwrap();
        let mut verifier = ForwardingVerifier::with_limits(credentials, 500, 8);
        assert_eq!(
            verifier.verify_payload(payload(&past.to_frame().unwrap()), 2_000),
            Err(ForwardingProtocolError::Expired)
        );
        assert_eq!(
            verifier.verify_payload(payload(&future.to_frame().unwrap()), 2_000),
            Err(ForwardingProtocolError::Expired)
        );
    }

    #[test]
    fn endpoint_bounds_are_enforced() {
        assert_eq!(
            PlatformEndpoint::unix_socket(""),
            Err(ForwardingProtocolError::InvalidEndpoint)
        );
        assert_eq!(
            PlatformEndpoint::windows_named_pipe(r"\\.\pipe\other"),
            Err(ForwardingProtocolError::InvalidEndpoint)
        );

        let credentials = ForwardingCredentials::generate().unwrap();
        let mut verifier = ForwardingVerifier::new(credentials);
        assert_eq!(
            verifier.verify_payload(&vec![b'x'; MAX_FORWARDING_FRAME_BYTES + 1], 0),
            Err(ForwardingProtocolError::TooLarge)
        );
    }
}
