use std::fmt;

use serde::{Serialize, Serializer};

pub const DIAGNOSTICS_SCHEMA_VERSION: u16 = 1;
pub const REDACTED_VALUE: &str = "[redacted]";

const EXCLUDED_FIELDS: &[&str] = &[
    "raw_prompt",
    "assistant_message",
    "command",
    "approval_arguments",
    "credential",
    "mac_secret",
    "inherited_environment_value",
];

const RETAINED_FIELDS: &[&str] = &[
    "component",
    "operation",
    "outcome",
    "error_code",
    "counter",
    "duration_ms",
    "event_id",
    "action_id",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticPrivacy {
    pub schema_version: u16,
    pub content_policy: &'static str,
    pub excluded_fields: &'static [&'static str],
    pub retained_fields: &'static [&'static str],
}

pub const fn diagnostic_privacy() -> DiagnosticPrivacy {
    DiagnosticPrivacy {
        schema_version: DIAGNOSTICS_SCHEMA_VERSION,
        content_policy: "metadata_only",
        excluded_fields: EXCLUDED_FIELDS,
        retained_fields: RETAINED_FIELDS,
    }
}

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub struct Redacted<T>(pub T);

impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED_VALUE)
    }
}

impl<T> fmt::Display for Redacted<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED_VALUE)
    }
}

impl<T> Serialize for Redacted<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(REDACTED_VALUE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_values_are_never_formatted_or_serialized() {
        for sensitive in [
            "raw-prompt-sentinel",
            "assistant-message-sentinel",
            "command-sentinel",
            "approval-arguments-sentinel",
            "credential-sentinel",
            "mac-secret-sentinel",
            "inherited-environment-sentinel",
        ] {
            let value = Redacted(sensitive);
            assert_eq!(format!("{value}"), REDACTED_VALUE);
            assert_eq!(format!("{value:?}"), REDACTED_VALUE);
            assert_eq!(serde_json::to_string(&value).unwrap(), r#""[redacted]""#);
        }
    }

    #[test]
    fn privacy_contract_names_every_excluded_content_class() {
        let privacy = diagnostic_privacy();
        assert_eq!(privacy.content_policy, "metadata_only");
        for field in [
            "raw_prompt",
            "assistant_message",
            "command",
            "approval_arguments",
            "credential",
            "mac_secret",
            "inherited_environment_value",
        ] {
            assert!(privacy.excluded_fields.contains(&field));
        }
    }
}
