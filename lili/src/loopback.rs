use std::{
    collections::HashMap,
    fmt::Write as _,
    io,
    net::{Ipv4Addr, TcpListener},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{
        HeaderMap, HeaderName, HeaderValue, Method, StatusCode,
        header::{
            CACHE_CONTROL, CONTENT_SECURITY_POLICY, COOKIE, HOST, ORIGIN, REFERRER_POLICY,
            SET_COOKIE, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
        },
    },
    middleware::{self, Next},
    response::{IntoResponse, Redirect, Response},
};
use axum_server::tls_rustls::RustlsConfig;
use hmac::{Hmac, KeyInit, Mac};
use rcgen::{CertifiedKey, generate_simple_self_signed};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;

const COOKIE_PREFIX: &str = "lili_loopback_";
const MAX_SIGNED_BODY_BYTES: usize = 1024 * 1024;
const PERMISSIONS_POLICY: &str = "accelerometer=(), camera=(), geolocation=(), gyroscope=(), magnetometer=(), microphone=(), payment=(), usb=()";
const SIGNATURE_TTL: Duration = Duration::from_secs(30);
const SIGNATURE_VERSION: &[u8] = b"lili-request-v1";

const CONTENT_SHA256_HEADER: &str = "x-lili-content-sha256";
const ISSUED_AT_HEADER: &str = "x-lili-issued-at";
const REQUEST_ID_HEADER: &str = "x-lili-request-id";
const SIGNATURE_HEADER: &str = "x-lili-signature";

pub struct LoopbackServer {
    bootstrap_url: tauri::Url,
    certificate_sha256: [u8; 32],
    listener: TcpListener,
    origin: tauri::Url,
    router: Router,
    signer: RequestSigner,
    tls_config: RustlsConfig,
}

impl LoopbackServer {
    pub fn bind(router: Router) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let authority = format!("127.0.0.1:{port}");
        let origin = format!("https://{authority}");
        let session_secret = random_bytes()?;
        let signing_secret = random_bytes()?;
        let instance_id = encode_hex(&random_bytes()?);
        let signer = RequestSigner::new(authority.clone(), instance_id, signing_secret);
        let security = LoopbackSecurity::new(
            authority,
            origin.clone(),
            encode_hex(&session_secret),
            signer.clone(),
        );
        let (tls_config, certificate_sha256) = ephemeral_tls_config()?;
        let bootstrap_url = format!("{origin}{}", security.bootstrap_path)
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let origin = origin
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

        Ok(Self {
            bootstrap_url,
            certificate_sha256,
            listener,
            origin,
            router: protect(router, security),
            signer,
            tls_config,
        })
    }

    pub fn bootstrap_url(&self) -> tauri::Url {
        self.bootstrap_url.clone()
    }

    pub fn certificate_sha256(&self) -> [u8; 32] {
        self.certificate_sha256
    }

    pub fn origin(&self) -> tauri::Url {
        self.origin.clone()
    }

    pub fn signer(&self) -> RequestSigner {
        self.signer.clone()
    }

    pub fn spawn(self, shutdown: oneshot::Receiver<()>) {
        tauri::async_runtime::spawn(async move {
            let handle = axum_server::Handle::new();
            let shutdown_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                let _ = shutdown.await;
                shutdown_handle.graceful_shutdown(Some(Duration::from_secs(2)));
            });
            let server = match axum_server::from_tcp_rustls(self.listener, self.tls_config) {
                Ok(server) => server,
                Err(_) => {
                    crate::diagnostics::error("loopback", "initialize", "listener_failed");
                    return;
                }
            };
            if server
                .handle(handle)
                .serve(self.router.into_make_service())
                .await
                .is_err()
            {
                crate::diagnostics::error("loopback", "serve", "transport_stopped");
            }
        });
    }
}

#[derive(Clone)]
pub struct RequestSigner {
    inner: Arc<RequestSignerInner>,
}

struct RequestSignerInner {
    authority: Arc<str>,
    instance_id: Arc<str>,
    replay_cache: Mutex<HashMap<String, Instant>>,
    secret: [u8; 32],
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedRequest {
    content_sha256: String,
    issued_at: u64,
    request_id: String,
    signature: String,
}

impl RequestSigner {
    fn new(authority: String, instance_id: String, secret: [u8; 32]) -> Self {
        Self {
            inner: Arc::new(RequestSignerInner {
                authority: authority.into(),
                instance_id: instance_id.into(),
                replay_cache: Mutex::new(HashMap::new()),
                secret,
            }),
        }
    }

    pub fn sign(
        &self,
        method: &str,
        path_and_query: &str,
        content_type: &str,
        body: &[u8],
    ) -> Result<SignedRequest, &'static str> {
        validate_signing_input(method, path_and_query, content_type, body)?;
        let issued_at = unix_timestamp().map_err(|_| "system clock is before Unix epoch")?;
        let request_id = encode_hex(&random_nonce().map_err(|_| "failed to generate request ID")?);
        let content_sha256 = Sha256::digest(body);
        let signature = self.compute_signature(
            method,
            path_and_query,
            content_type,
            issued_at,
            &request_id,
            &content_sha256,
        );
        Ok(SignedRequest {
            content_sha256: encode_hex(&content_sha256),
            issued_at,
            request_id,
            signature: encode_hex(&signature),
        })
    }

    fn compute_signature(
        &self,
        method: &str,
        path_and_query: &str,
        content_type: &str,
        issued_at: u64,
        request_id: &str,
        content_sha256: &[u8],
    ) -> Vec<u8> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.inner.secret)
            .expect("HMAC accepts a key of any size");
        for field in [
            SIGNATURE_VERSION,
            self.inner.instance_id.as_bytes(),
            method.as_bytes(),
            self.inner.authority.as_bytes(),
            path_and_query.as_bytes(),
            content_type.as_bytes(),
            issued_at.to_string().as_bytes(),
            request_id.as_bytes(),
            content_sha256,
        ] {
            mac.update(&(field.len() as u64).to_be_bytes());
            mac.update(field);
        }
        mac.finalize().into_bytes().to_vec()
    }

    fn verify(&self, request: &Request, body: &[u8]) -> Result<(), &'static str> {
        let headers = request.headers();
        let request_id = required_header(headers, REQUEST_ID_HEADER)?;
        let issued_at = required_header(headers, ISSUED_AT_HEADER)?
            .parse::<u64>()
            .map_err(|_| "invalid request timestamp")?;
        validate_fresh_timestamp(issued_at)?;
        if request_id.len() != 32 || !request_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("invalid request ID");
        }
        let content_sha256 = Sha256::digest(body);
        if !constant_time_eq(
            required_header(headers, CONTENT_SHA256_HEADER)?.as_bytes(),
            encode_hex(&content_sha256).as_bytes(),
        ) {
            return Err("request body digest mismatch");
        }
        let content_type = headers
            .get(axum::http::header::CONTENT_TYPE)
            .map(|value| value.to_str().map_err(|_| "invalid content type"))
            .transpose()?
            .unwrap_or_default();
        let expected = self.compute_signature(
            request.method().as_str(),
            request
                .uri()
                .path_and_query()
                .map_or(request.uri().path(), |value| value.as_str()),
            content_type,
            issued_at,
            request_id,
            &content_sha256,
        );
        let signature = decode_hex(required_header(headers, SIGNATURE_HEADER)?)
            .ok_or("invalid request signature")?;
        if !constant_time_eq(&signature, &expected) {
            return Err("invalid request signature");
        }
        let now = Instant::now();
        let mut replay_cache = self
            .inner
            .replay_cache
            .lock()
            .map_err(|_| "request replay cache is unavailable")?;
        replay_cache.retain(|_, accepted_at| now.duration_since(*accepted_at) <= SIGNATURE_TTL);
        if replay_cache.insert(request_id.to_owned(), now).is_some() {
            return Err("request signature was already used");
        }
        Ok(())
    }
}

#[derive(Clone)]
struct LoopbackSecurity {
    authority: Arc<str>,
    bootstrap_available: Arc<AtomicBool>,
    bootstrap_path: Arc<str>,
    cookie_name: Arc<str>,
    csp: HeaderValue,
    origin: Arc<str>,
    secret: Arc<str>,
    signer: RequestSigner,
}

impl LoopbackSecurity {
    fn new(authority: String, origin: String, secret: String, signer: RequestSigner) -> Self {
        let cookie_name = format!("{COOKIE_PREFIX}{}", &secret[..16]);
        let csp = HeaderValue::from_str(&format!(
            "default-src 'self'; base-uri 'none'; connect-src 'self' wss://{authority}; font-src 'self'; form-action 'self'; frame-ancestors 'none'; img-src 'self' data:; object-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'"
        ))
        .expect("generated CSP is valid");
        Self {
            authority: authority.into(),
            bootstrap_available: Arc::new(AtomicBool::new(true)),
            bootstrap_path: format!("/_lili/bootstrap/{secret}").into(),
            cookie_name: cookie_name.into(),
            csp,
            origin: origin.into(),
            secret: secret.into(),
            signer,
        }
    }

    fn harden(&self, mut response: Response) -> Response {
        let headers = response.headers_mut();
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(CONTENT_SECURITY_POLICY, self.csp.clone());
        headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
        headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
        headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
        headers.insert(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static(PERMISSIONS_POLICY),
        );
        response
    }
}

fn protect(router: Router, security: LoopbackSecurity) -> Router {
    router.layer(middleware::from_fn_with_state(security, authorize))
}

async fn authorize(
    State(security): State<LoopbackSecurity>,
    mut request: Request,
    next: Next,
) -> Response {
    if !header_matches(request.headers(), HOST, &security.authority) {
        return security.harden(StatusCode::MISDIRECTED_REQUEST.into_response());
    }
    if request.uri().path() == security.bootstrap_path.as_ref() {
        if request.method() != Method::GET {
            return security.harden(StatusCode::METHOD_NOT_ALLOWED.into_response());
        }
        if security
            .bootstrap_available
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return security.harden(StatusCode::GONE.into_response());
        }
        let mut response = Redirect::to("/").into_response();
        let cookie = format!(
            "{}={}; HttpOnly; Secure; SameSite=Strict; Path=/",
            security.cookie_name, security.secret
        );
        response.headers_mut().insert(
            SET_COOKIE,
            HeaderValue::from_str(&cookie).expect("generated cookie is valid"),
        );
        return security.harden(response);
    }
    if !has_session_cookie(request.headers(), &security.cookie_name, &security.secret) {
        return security.harden(StatusCode::UNAUTHORIZED.into_response());
    }
    if requires_origin(request.method())
        && !header_matches(request.headers(), ORIGIN, &security.origin)
    {
        return security.harden(StatusCode::FORBIDDEN.into_response());
    }
    if request.uri().path().starts_with("/api/") {
        let (parts, body) = request.into_parts();
        let body = match to_bytes(body, MAX_SIGNED_BODY_BYTES).await {
            Ok(body) => body,
            Err(_) => return security.harden(StatusCode::PAYLOAD_TOO_LARGE.into_response()),
        };
        request = Request::from_parts(parts, Body::from(body.clone()));
        if security.signer.verify(&request, &body).is_err() {
            return security.harden(StatusCode::UNAUTHORIZED.into_response());
        }
    }
    security.harden(next.run(request).await)
}

fn requires_origin(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

fn has_session_cookie(headers: &HeaderMap, expected_name: &str, expected_value: &str) -> bool {
    headers.get_all(COOKIE).iter().any(|value| {
        value.to_str().ok().is_some_and(|cookies| {
            cookies.split(';').any(|cookie| {
                cookie.trim().split_once('=').is_some_and(|(name, value)| {
                    name == expected_name
                        && constant_time_eq(value.as_bytes(), expected_value.as_bytes())
                })
            })
        })
    })
}

fn header_matches(headers: &HeaderMap, name: HeaderName, expected: &str) -> bool {
    headers
        .get(name)
        .is_some_and(|value| constant_time_eq(value.as_bytes(), expected.as_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0, |difference, (left, right)| difference | (left ^ right))
            == 0
}

fn validate_signing_input(
    method: &str,
    path_and_query: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), &'static str> {
    if !matches!(method, "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE") {
        return Err("unsupported method");
    }
    if !path_and_query.starts_with("/api/")
        || path_and_query.contains('#')
        || path_and_query.contains(['\r', '\n'])
        || path_and_query.parse::<axum::http::Uri>().is_err()
    {
        return Err("invalid API path");
    }
    if content_type.len() > 256 || !content_type.is_ascii() {
        return Err("invalid content type");
    }
    if body.len() > MAX_SIGNED_BODY_BYTES {
        return Err("request body is too large");
    }
    Ok(())
}

fn validate_fresh_timestamp(issued_at: u64) -> Result<(), &'static str> {
    let now = unix_timestamp().map_err(|_| "system clock is before Unix epoch")?;
    (now.abs_diff(issued_at) <= SIGNATURE_TTL.as_secs())
        .then_some(())
        .ok_or("request signature is stale")
}

fn required_header<'a>(
    headers: &'a HeaderMap,
    name: &'static str,
) -> Result<&'a str, &'static str> {
    headers
        .get(name)
        .ok_or("missing request signature")?
        .to_str()
        .map_err(|_| "invalid request signature")
}

fn unix_timestamp() -> Result<u64, std::time::SystemTimeError> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}

fn random_bytes() -> io::Result<[u8; 32]> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes)
}

fn random_nonce() -> io::Result<[u8; 16]> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes)
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a string cannot fail");
    }
    encoded
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

fn ephemeral_tls_config() -> io::Result<(RustlsConfig, [u8; 32])> {
    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).map_err(io::Error::other)?;
    let certificate_der = cert.der().to_vec();
    let certificate_sha256: [u8; 32] = Sha256::digest(&certificate_der).into();
    let private_key_der = signing_key.serialize_der();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(certificate_der)],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(private_key_der)),
        )
        .map_err(io::Error::other)?;
    Ok((
        RustlsConfig::from_config(Arc::new(config)),
        certificate_sha256,
    ))
}

#[cfg(test)]
mod tests {
    use axum::{body::Body, http::Request, routing::any};
    use tower::ServiceExt;

    use super::*;

    const AUTHORITY: &str = "127.0.0.1:43123";
    const ORIGIN_VALUE: &str = "https://127.0.0.1:43123";
    const SECRET: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn app() -> (Router, LoopbackSecurity) {
        let signer =
            RequestSigner::new(AUTHORITY.to_owned(), "test-instance".to_owned(), [0x5a; 32]);
        let security = LoopbackSecurity::new(
            AUTHORITY.to_owned(),
            ORIGIN_VALUE.to_owned(),
            SECRET.to_owned(),
            signer,
        );
        let router = Router::new()
            .route("/", any(|| async { StatusCode::NO_CONTENT }))
            .route(
                "/pet-assets/opaque-id",
                any(|| async { StatusCode::NO_CONTENT }),
            )
            .route("/api/v1/snapshot", any(|| async { StatusCode::NO_CONTENT }));
        (protect(router, security.clone()), security)
    }

    fn cookie() -> String {
        format!("{COOKIE_PREFIX}{}={SECRET}", &SECRET[..16])
    }

    #[tokio::test]
    async fn bootstrap_is_single_use_and_sets_secure_cookie() {
        let (app, security) = app();
        let request = || {
            Request::get(security.bootstrap_path.as_ref())
                .header(HOST, AUTHORITY)
                .body(Body::empty())
                .unwrap()
        };
        let response = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let value = response.headers()[SET_COOKIE].to_str().unwrap();
        assert!(value.contains("HttpOnly"));
        assert!(value.contains("Secure"));
        assert_eq!(
            app.oneshot(request()).await.unwrap().status(),
            StatusCode::GONE
        );
    }

    #[tokio::test]
    async fn protected_route_requires_cookie() {
        let (app, _) = app();
        let response = app
            .oneshot(
                Request::get("/")
                    .header(HOST, AUTHORITY)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn authenticated_asset_request_does_not_require_api_signature() {
        let (app, _) = app();
        let response = app
            .oneshot(
                Request::get("/pet-assets/opaque-id")
                    .header(HOST, AUTHORITY)
                    .header(COOKIE, cookie())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn authenticated_api_get_still_requires_signature() {
        let (app, _) = app();
        let response = app
            .oneshot(
                Request::get("/api/v1/snapshot")
                    .header(HOST, AUTHORITY)
                    .header(COOKIE, cookie())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn signed_request_cannot_be_replayed() {
        let signer =
            RequestSigner::new(AUTHORITY.to_owned(), "test-instance".to_owned(), [0x5a; 32]);
        let signed = signer
            .sign("POST", "/api/v1/interactions", "application/json", b"{}")
            .unwrap();
        let request = || {
            Request::post("/api/v1/interactions")
                .header(HOST, AUTHORITY)
                .header(ORIGIN, ORIGIN_VALUE)
                .header(COOKIE, cookie())
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .header(CONTENT_SHA256_HEADER, &signed.content_sha256)
                .header(ISSUED_AT_HEADER, signed.issued_at.to_string())
                .header(REQUEST_ID_HEADER, &signed.request_id)
                .header(SIGNATURE_HEADER, &signed.signature)
                .body(Body::from("{}"))
                .unwrap()
        };
        let security = LoopbackSecurity::new(
            AUTHORITY.to_owned(),
            ORIGIN_VALUE.to_owned(),
            SECRET.to_owned(),
            signer,
        );
        let app = protect(
            Router::new().route(
                "/api/v1/interactions",
                any(|| async { StatusCode::NO_CONTENT }),
            ),
            security,
        );
        assert_eq!(
            app.clone().oneshot(request()).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            app.oneshot(request()).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
