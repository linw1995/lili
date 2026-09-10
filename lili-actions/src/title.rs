use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ActionTrigger, MAX_ACTION_OUTPUT_BYTES};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTitleRequest {
    pub version: u16,
    pub request_id: Uuid,
    pub trigger: ActionTrigger,
    pub provider: String,
    pub session_id: String,
}

impl SessionTitleRequest {
    pub fn new(provider: String, session_id: String) -> Self {
        Self {
            version: 1,
            request_id: Uuid::new_v4(),
            trigger: ActionTrigger::SessionTitle,
            provider,
            session_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum TitleResponseError {
    #[error("title response exceeds its byte limit")]
    TooLarge,
    #[error("invalid title response protocol")]
    InvalidProtocol,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    version: u16,
    title: serde_json::Value,
}

pub fn decode_title_response(bytes: &[u8]) -> Result<Option<String>, TitleResponseError> {
    if bytes.len() > MAX_ACTION_OUTPUT_BYTES {
        return Err(TitleResponseError::TooLarge);
    }
    let response: Response =
        serde_json::from_slice(bytes).map_err(|_| TitleResponseError::InvalidProtocol)?;
    if response.version != 1 {
        return Err(TitleResponseError::InvalidProtocol);
    }
    let title = match response.title {
        serde_json::Value::Null => return Ok(None),
        serde_json::Value::String(title) => title,
        _ => return Err(TitleResponseError::InvalidProtocol),
    };
    let title: String = title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect();
    let title = title.trim().to_owned();
    Ok((!title.is_empty()).then_some(title))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_required_fields_and_single_document() {
        for input in [
            r#"{"version":1}"#,
            r#"{"version":2,"title":"a"}"#,
            r#"{"version":1,"title":1}"#,
            r#"{"version":1,"title":null,"extra":true}"#,
            r#"{"version":1,"title":null} {}"#,
        ] {
            assert!(decode_title_response(input.as_bytes()).is_err(), "{input}");
        }
        assert!(decode_title_response(&[0xff]).is_err());
        assert!(decode_title_response(&vec![b' '; MAX_ACTION_OUTPUT_BYTES + 1]).is_err());
        assert_eq!(
            decode_title_response(br#"{"version":1,"title":null}"#),
            Ok(None)
        );
    }

    #[test]
    fn normalizes_and_bounds_unicode_as_plain_text() {
        let input = serde_json::json!({"version": 1, "title": "  <b>hello</b>\n\tworld\u{0}  "});
        assert_eq!(
            decode_title_response(&serde_json::to_vec(&input).unwrap()),
            Ok(Some("<b>hello</b> world".into()))
        );
        let input = serde_json::json!({"version": 1, "title": "🦀".repeat(300)});
        assert_eq!(
            decode_title_response(&serde_json::to_vec(&input).unwrap())
                .unwrap()
                .unwrap()
                .chars()
                .count(),
            256
        );
    }
}
