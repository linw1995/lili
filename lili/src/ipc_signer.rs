use std::borrow::Cow;

use tauri::ipc::{InvokeBody, Request};

use crate::loopback::{RequestSigner, SignedRequest};

const CONTENT_TYPE_HEADER: &str = "x-lili-sign-content-type";
const METHOD_HEADER: &str = "x-lili-sign-method";
const PATH_HEADER: &str = "x-lili-sign-path";

pub const FETCH_SIGNER_SCRIPT: &str = r#"
(() => {
  const nativeFetch = window.fetch.bind(window);
  window.fetch = async (input, init) => {
    const request = new Request(input, init);
    const url = new URL(request.url);
    if (url.origin !== window.location.origin || !url.pathname.startsWith('/api/')) {
      return nativeFetch(request);
    }
    const body = new Uint8Array(await request.clone().arrayBuffer());
    const contentType = request.headers.get('content-type') || '';
    const signed = await window.__TAURI_INTERNALS__.invoke('sign_loopback_request', body, {
      headers: {
        'x-lili-sign-method': request.method,
        'x-lili-sign-path': url.pathname + url.search,
        'x-lili-sign-content-type': contentType
      }
    });
    const headers = new Headers(request.headers);
    headers.set('x-lili-content-sha256', signed.contentSha256);
    headers.set('x-lili-issued-at', String(signed.issuedAt));
    headers.set('x-lili-request-id', signed.requestId);
    headers.set('x-lili-signature', signed.signature);
    return nativeFetch(new Request(request, { headers }));
  };
})();
"#;

#[tauri::command]
pub fn sign_loopback_request(
    request: Request<'_>,
    signer: tauri::State<'_, RequestSigner>,
) -> Result<SignedRequest, String> {
    let method = metadata(&request, METHOD_HEADER)?;
    let path_and_query = metadata(&request, PATH_HEADER)?;
    let content_type = metadata(&request, CONTENT_TYPE_HEADER)?;
    let body = signing_body(request.body())?;
    signer
        .sign(method, path_and_query, content_type, &body)
        .map_err(str::to_owned)
}

fn signing_body(body: &InvokeBody) -> Result<Cow<'_, [u8]>, String> {
    match body {
        InvokeBody::Raw(body) => Ok(Cow::Borrowed(body)),
        InvokeBody::Json(value) => {
            let bytes = value
                .as_array()
                .ok_or_else(|| "the signer requires a byte-array request body".to_owned())?;
            let body = bytes
                .iter()
                .map(|byte| {
                    byte.as_u64()
                        .filter(|byte| *byte <= u64::from(u8::MAX))
                        .map(|byte| byte as u8)
                        .ok_or_else(|| "the signer body contains a non-byte value".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Cow::Owned(body))
        }
    }
}

fn metadata<'a>(request: &'a Request<'_>, name: &'static str) -> Result<&'a str, String> {
    request
        .headers()
        .get(name)
        .ok_or_else(|| format!("missing signer metadata: {name}"))?
        .to_str()
        .map_err(|_| format!("invalid signer metadata: {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_body_accepts_raw_and_json_bytes_only() {
        let raw = InvokeBody::Raw(vec![0, 1, 255]);
        assert_eq!(signing_body(&raw).unwrap().as_ref(), [0, 1, 255]);

        let json = InvokeBody::Json(serde_json::json!([0, 1, 255]));
        assert_eq!(signing_body(&json).unwrap().as_ref(), [0, 1, 255]);

        let object = InvokeBody::Json(serde_json::json!({"byte": 1}));
        assert!(signing_body(&object).is_err());

        let invalid_byte = InvokeBody::Json(serde_json::json!([256]));
        assert!(signing_body(&invalid_byte).is_err());
    }
}
