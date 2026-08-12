#[cfg(target_os = "macos")]
mod macos {
    use std::ptr;

    use block2::DynBlock;
    use objc2::{
        ClassType, DefinedClass, MainThreadOnly, define_class,
        ffi::{OBJC_ASSOCIATION_RETAIN_NONATOMIC, objc_setAssociatedObject},
        msg_send,
        rc::Retained,
        runtime::{AnyObject, NSObject, ProtocolObject},
    };
    use objc2_foundation::{
        MainThreadMarker, NSObjectProtocol, NSString, NSURL, NSURLAuthenticationChallenge,
        NSURLAuthenticationMethodServerTrust, NSURLCredential, NSURLRequest,
        NSURLSessionAuthChallengeDisposition,
    };
    use objc2_security::SecTrust;
    use objc2_web_kit::{
        WKDownload, WKNavigation, WKNavigationAction, WKNavigationActionPolicy,
        WKNavigationDelegate, WKNavigationResponse, WKNavigationResponsePolicy, WKWebView,
    };
    use sha2::{Digest, Sha256};
    use tauri::WebviewWindow;

    static PINNING_DELEGATE_ASSOCIATION_KEY: u8 = 0;

    struct PinningNavigationDelegateIvars {
        certificate_sha256: [u8; 32],
        host: String,
        original: Retained<ProtocolObject<dyn WKNavigationDelegate>>,
        port: u16,
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[ivars = PinningNavigationDelegateIvars]
        struct PinningNavigationDelegate;

        unsafe impl NSObjectProtocol for PinningNavigationDelegate {}

        unsafe impl WKNavigationDelegate for PinningNavigationDelegate {
            #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
            fn navigation_policy(
                &self,
                webview: &WKWebView,
                action: &WKNavigationAction,
                handler: &DynBlock<dyn Fn(WKNavigationActionPolicy)>,
            ) {
                unsafe {
                    let _: () = msg_send![
                        &*self.ivars().original,
                        webView: webview,
                        decidePolicyForNavigationAction: action,
                        decisionHandler: handler
                    ];
                }
            }

            #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
            fn navigation_policy_response(
                &self,
                webview: &WKWebView,
                response: &WKNavigationResponse,
                handler: &DynBlock<dyn Fn(WKNavigationResponsePolicy)>,
            ) {
                unsafe {
                    let _: () = msg_send![
                        &*self.ivars().original,
                        webView: webview,
                        decidePolicyForNavigationResponse: response,
                        decisionHandler: handler
                    ];
                }
            }

            #[unsafe(method(webView:didFinishNavigation:))]
            fn did_finish_navigation(&self, webview: &WKWebView, navigation: &WKNavigation) {
                unsafe {
                    let _: () = msg_send![
                        &*self.ivars().original,
                        webView: webview,
                        didFinishNavigation: navigation
                    ];
                }
            }

            #[unsafe(method(webView:didCommitNavigation:))]
            fn did_commit_navigation(&self, webview: &WKWebView, navigation: &WKNavigation) {
                unsafe {
                    let _: () = msg_send![
                        &*self.ivars().original,
                        webView: webview,
                        didCommitNavigation: navigation
                    ];
                }
            }

            #[unsafe(method(webView:navigationAction:didBecomeDownload:))]
            fn navigation_download_action(
                &self,
                webview: &WKWebView,
                action: &WKNavigationAction,
                download: &WKDownload,
            ) {
                unsafe {
                    let _: () = msg_send![
                        &*self.ivars().original,
                        webView: webview,
                        navigationAction: action,
                        didBecomeDownload: download
                    ];
                }
            }

            #[unsafe(method(webView:navigationResponse:didBecomeDownload:))]
            fn navigation_download_response(
                &self,
                webview: &WKWebView,
                response: &WKNavigationResponse,
                download: &WKDownload,
            ) {
                unsafe {
                    let _: () = msg_send![
                        &*self.ivars().original,
                        webView: webview,
                        navigationResponse: response,
                        didBecomeDownload: download
                    ];
                }
            }

            #[unsafe(method(webViewWebContentProcessDidTerminate:))]
            fn web_content_process_did_terminate(&self, webview: &WKWebView) {
                unsafe {
                    let _: () = msg_send![
                        &*self.ivars().original,
                        webViewWebContentProcessDidTerminate: webview
                    ];
                }
            }

            #[unsafe(method(webView:didReceiveAuthenticationChallenge:completionHandler:))]
            fn authentication_challenge(
                &self,
                _webview: &WKWebView,
                challenge: &NSURLAuthenticationChallenge,
                handler: &DynBlock<
                    dyn Fn(NSURLSessionAuthChallengeDisposition, *mut NSURLCredential),
                >,
            ) {
                let credential = unsafe { self.pinned_credential(challenge) };
                match credential {
                    Some(credential) => handler.call((
                        NSURLSessionAuthChallengeDisposition::UseCredential,
                        credential,
                    )),
                    None => handler.call((
                        NSURLSessionAuthChallengeDisposition::CancelAuthenticationChallenge,
                        ptr::null_mut(),
                    )),
                }
            }
        }
    );

    impl PinningNavigationDelegate {
        fn new(
            original: Retained<ProtocolObject<dyn WKNavigationDelegate>>,
            certificate_sha256: [u8; 32],
            host: String,
            port: u16,
            mtm: MainThreadMarker,
        ) -> Retained<Self> {
            let delegate = mtm
                .alloc::<Self>()
                .set_ivars(PinningNavigationDelegateIvars {
                    certificate_sha256,
                    host,
                    original,
                    port,
                });
            unsafe { msg_send![super(delegate), init] }
        }

        unsafe fn pinned_credential(
            &self,
            challenge: &NSURLAuthenticationChallenge,
        ) -> Option<*mut NSURLCredential> {
            let protection_space = challenge.protectionSpace();
            if !self.matches_protection_space(&protection_space) {
                return None;
            }
            let trust: *mut SecTrust = unsafe { msg_send![&*protection_space, serverTrust] };
            let trust = unsafe { trust.as_ref() }?;
            unsafe { self.credential_for_trust(trust) }
        }

        fn matches_protection_space(
            &self,
            protection_space: &objc2_foundation::NSURLProtectionSpace,
        ) -> bool {
            &*protection_space.authenticationMethod()
                == unsafe { NSURLAuthenticationMethodServerTrust }
                && protection_space.host().to_string() == self.ivars().host
                && protection_space.port() == self.ivars().port as isize
        }

        unsafe fn credential_for_trust(&self, trust: &SecTrust) -> Option<*mut NSURLCredential> {
            #[allow(deprecated)]
            let certificate = unsafe { trust.certificate_at_index(0) }?;
            let certificate_der = unsafe { certificate.data() };
            let actual_sha256: [u8; 32] = Sha256::digest(certificate_der.to_vec()).into();
            if actual_sha256 != self.ivars().certificate_sha256 {
                return None;
            }
            let credential: *mut NSURLCredential =
                unsafe { msg_send![NSURLCredential::class(), credentialForTrust: trust] };
            (!credential.is_null()).then_some(credential)
        }
    }

    pub fn install_and_navigate(
        window: &WebviewWindow,
        bootstrap_url: tauri::Url,
        certificate_sha256: [u8; 32],
    ) -> Result<(), String> {
        let host = bootstrap_url
            .host_str()
            .ok_or_else(|| "loopback URL has no host".to_owned())?
            .to_owned();
        let port = bootstrap_url
            .port()
            .ok_or_else(|| "loopback URL has no port".to_owned())?;
        let url = bootstrap_url.to_string();
        window
            .with_webview(move |platform_webview| unsafe {
                let webview = &*(platform_webview.inner().cast::<WKWebView>());
                let Some(original) = webview.navigationDelegate() else {
                    crate::diagnostics::error("tls_pinning", "install", "missing_delegate");
                    return;
                };
                let Some(mtm) = MainThreadMarker::new() else {
                    crate::diagnostics::error("tls_pinning", "install", "wrong_thread");
                    return;
                };
                let delegate =
                    PinningNavigationDelegate::new(original, certificate_sha256, host, port, mtm);
                webview.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                objc_setAssociatedObject(
                    webview as *const WKWebView as *mut AnyObject,
                    &PINNING_DELEGATE_ASSOCIATION_KEY as *const u8 as *const _,
                    Retained::as_ptr(&delegate).cast::<AnyObject>().cast_mut(),
                    OBJC_ASSOCIATION_RETAIN_NONATOMIC,
                );
                let Some(url) = NSURL::URLWithString(&NSString::from_str(&url)) else {
                    crate::diagnostics::error("tls_pinning", "navigate", "invalid_url");
                    return;
                };
                let request = NSURLRequest::requestWithURL(&url);
                let _ = webview.loadRequest(&request);
            })
            .map_err(|error| error.to_string())
    }
}

#[cfg(target_os = "macos")]
pub use macos::install_and_navigate;

#[cfg(target_os = "windows")]
mod windows {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use sha2::{Digest, Sha256};
    use tauri::WebviewWindow;
    use webview2_com::{CoTaskMemPWSTR, ServerCertificateErrorDetectedEventHandler};
    use windows::core::{HSTRING, Interface, PWSTR};

    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW,
        COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL, ICoreWebView2_14,
    };

    pub fn install_and_navigate(
        window: &WebviewWindow,
        bootstrap_url: tauri::Url,
        certificate_sha256: [u8; 32],
    ) -> Result<(), String> {
        let expected_origin = bootstrap_url.origin().ascii_serialization();
        let url = bootstrap_url.to_string();
        window
            .with_webview(move |platform_webview| unsafe {
                let controller = platform_webview.controller();
                let webview = match controller.CoreWebView2() {
                    Ok(webview) => webview,
                    Err(_) => {
                        crate::diagnostics::error("tls_pinning", "install", "missing_webview");
                        return;
                    }
                };
                let webview14 = match webview.cast::<ICoreWebView2_14>() {
                    Ok(webview) => webview,
                    Err(_) => {
                        crate::diagnostics::error("tls_pinning", "install", "runtime_too_old");
                        return;
                    }
                };
                let handler = ServerCertificateErrorDetectedEventHandler::create(Box::new(
                    move |_, arguments| {
                        let Some(arguments) = arguments else {
                            return Ok(());
                        };
                        let mut request_uri = PWSTR::null();
                        arguments.RequestUri(&mut request_uri)?;
                        let request_uri = CoTaskMemPWSTR::from(request_uri).to_string();
                        let origin_matches = tauri::Url::parse(&request_uri)
                            .is_ok_and(|uri| uri.origin().ascii_serialization() == expected_origin);
                        let certificate_matches = arguments
                            .ServerCertificate()
                            .and_then(|certificate| {
                                let mut pem = PWSTR::null();
                                certificate.ToPemEncoding(&mut pem)?;
                                Ok(CoTaskMemPWSTR::from(pem).to_string())
                            })
                            .ok()
                            .and_then(|pem| decode_pem_certificate(&pem))
                            .is_some_and(|der| {
                                let actual: [u8; 32] = Sha256::digest(der).into();
                                actual == certificate_sha256
                            });
                        arguments.SetAction(if origin_matches && certificate_matches {
                            COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_ALWAYS_ALLOW
                        } else {
                            COREWEBVIEW2_SERVER_CERTIFICATE_ERROR_ACTION_CANCEL
                        })
                    },
                ));
                let mut token = 0;
                if webview14
                    .add_ServerCertificateErrorDetected(&handler, &mut token)
                    .is_err()
                {
                    crate::diagnostics::error("tls_pinning", "install", "handler_failed");
                    return;
                }
                if webview.Navigate(&HSTRING::from(url)).is_err() {
                    crate::diagnostics::error("tls_pinning", "navigate", "navigation_failed");
                }
            })
            .map_err(|error| error.to_string())
    }

    fn decode_pem_certificate(pem: &str) -> Option<Vec<u8>> {
        let encoded = pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect::<String>();
        STANDARD.decode(encoded).ok()
    }
}

#[cfg(target_os = "windows")]
pub use windows::install_and_navigate;

#[cfg(target_os = "linux")]
mod linux {
    use gio::prelude::TlsCertificateExt;
    use sha2::{Digest, Sha256};
    use tauri::WebviewWindow;
    use webkit2gtk::{WebContextExt, WebViewExt};

    pub fn install_and_navigate(
        window: &WebviewWindow,
        bootstrap_url: tauri::Url,
        certificate_sha256: [u8; 32],
    ) -> Result<(), String> {
        let expected_origin = bootstrap_url.origin().ascii_serialization();
        let host = bootstrap_url
            .host_str()
            .ok_or_else(|| "loopback URL has no host".to_owned())?
            .to_owned();
        let url = bootstrap_url.to_string();
        window
            .with_webview(move |platform_webview| {
                let webview = platform_webview.inner();
                let retry_url = url.clone();
                webview.connect_load_failed_with_tls_errors(
                    move |webview, failing_uri, certificate, _| {
                        let origin_matches = tauri::Url::parse(failing_uri)
                            .is_ok_and(|uri| uri.origin().ascii_serialization() == expected_origin);
                        let certificate_matches = certificate.certificate().is_some_and(|der| {
                            let actual: [u8; 32] = Sha256::digest(der.as_ref()).into();
                            actual == certificate_sha256
                        });
                        if !(origin_matches && certificate_matches) {
                            return false;
                        }
                        let Some(context) = webview.context() else {
                            crate::diagnostics::error("tls_pinning", "install", "missing_context");
                            return false;
                        };
                        context.allow_tls_certificate_for_host(certificate, &host);
                        webview.load_uri(&retry_url);
                        true
                    },
                );
                webview.load_uri(&url);
            })
            .map_err(|error| error.to_string())
    }
}

#[cfg(target_os = "linux")]
pub use linux::install_and_navigate;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub fn install_and_navigate(
    _window: &tauri::WebviewWindow,
    _bootstrap_url: tauri::Url,
    _certificate_sha256: [u8; 32],
) -> Result<(), String> {
    Err("TLS certificate pinning is not implemented on this desktop platform".to_owned())
}
