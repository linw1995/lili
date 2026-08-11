mod normalization;
mod reducer;
mod types;

pub use normalization::{
    MAX_PROVIDER_PAYLOAD_BYTES, NormalizationError, normalize_json, normalize_provider_input,
};
pub use reducer::{DEFAULT_MINIMUM_DWELL_MS, ReductionOutcome, SessionReducer};
pub use types::{
    DisplayProjectContext, DisplaySummary, DisplayValueError, EventId, IdentityError,
    NormalizedSessionEvent, Notification, NotificationId, NotificationKind, NotificationState,
    PresentationState, ProviderCapabilitiesInputV1, ProviderId, ProviderInputV1,
    ProviderProjectInputV1, SessionEventKind, SessionId, SessionPhase, SessionSummary,
    SessionViewSnapshot, SourceCapabilities, TurnId,
};

pub const SESSION_SCHEMA_VERSION: u16 = 1;
