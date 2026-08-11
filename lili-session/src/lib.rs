mod codex;
mod forwarding;
mod normalization;
mod reducer;
mod spool;
mod transport;
mod types;

pub use codex::{
    CodexAdapterDiagnostics, CodexIntegrationSurface, LastAcceptedCodexEvent,
    MissingLifecycleCoverage, TESTED_CODEX_VERSION, normalize_hook_json, normalize_lifecycle_json,
    normalize_notify_json,
};
pub use forwarding::{
    DEFAULT_NONCE_CAPACITY, DEFAULT_REPLAY_WINDOW_MS, FORWARDING_PROTOCOL_VERSION, ForwardingAck,
    ForwardingAckDisposition, ForwardingCredentialRecord, ForwardingCredentials, ForwardingMessage,
    ForwardingProtocolError, ForwardingVerifier, MAX_FORWARDING_FRAME_BYTES, PlatformEndpoint,
    VerifiedForwardingMessage,
};
pub use normalization::{
    MAX_PROVIDER_PAYLOAD_BYTES, NormalizationError, normalize_json, normalize_provider_input,
};
pub use reducer::{
    DEFAULT_MINIMUM_DWELL_MS, ReducerRestoreError, ReductionOutcome, SessionReducer,
    SessionReducerState,
};
pub use spool::{
    ClaimedSpoolRecord, MAX_SPOOL_RECORD_BYTES, SpoolEnqueueOutcome, SpoolError, SpoolLimits,
    SpoolMetrics, SpoolStore, decode_spool_record,
};
pub use transport::{
    BoundForwardingEndpoint, ForwardingConnection, ForwardingCredentialStore,
    ForwardingTransportError, deliver_forwarding_message, private_forwarding_endpoint_is_live,
};
pub use types::{
    DisplayProjectContext, DisplaySummary, DisplayValueError, EventId, IdentityError,
    NormalizedEventValidationError, NormalizedSessionEvent, Notification, NotificationId,
    NotificationKind, NotificationState, PresentationState, ProviderCapabilitiesInputV1,
    ProviderId, ProviderInputV1, ProviderProjectInputV1, SessionEventKind, SessionId, SessionPhase,
    SessionSummary, SessionViewSnapshot, SourceCapabilities, TurnId,
};

pub const SESSION_SCHEMA_VERSION: u16 = 1;
