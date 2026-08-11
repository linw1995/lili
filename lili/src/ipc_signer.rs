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
    let json_body;
    let body = match request.body() {
        InvokeBody::Raw(body) => body.as_slice(),
        InvokeBody::Json(value) => {
            let bytes = value
                .as_array()
                .ok_or_else(|| "the signer requires a byte-array request body".to_owned())?;
            json_body = bytes
                .iter()
                .map(|byte| {
                    byte.as_u64()
                        .filter(|byte| *byte <= u8::MAX.into())
                        .map(|byte| byte as u8)
                        .ok_or_else(|| "the signer body contains a non-byte value".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            &json_body
        }
    };
    signer
        .sign(method, path_and_query, content_type, body)
        .map_err(str::to_owned)
}

fn metadata<'a>(request: &'a Request<'_>, name: &'static str) -> Result<&'a str, String> {
    request
        .headers()
        .get(name)
        .ok_or_else(|| format!("missing signer metadata: {name}"))?
        .to_str()
        .map_err(|_| format!("invalid signer metadata: {name}"))
}
