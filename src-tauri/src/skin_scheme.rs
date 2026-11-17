//! The `skin://` scheme (plan E7.7.2, shell-local per ADR 0020): one
//! asynchronous URI scheme whose every response is the Host's answer to
//! `IpcRequest::SkinFile` for the URL's path, copied unchanged. The Host
//! decides what exists and how long it may be cached
//! (`rekindle_skin::serve`); the shell only speaks HTTP.
//!
//! Origins: `skin://localhost` on macOS and Linux, `http://skin.localhost`
//! on Windows. The CSP names both (`tauri.conf.json`).

use std::sync::Arc;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_types::dto::skin_library::SkinServed;
use rekindle_types::dto::{CommandError, Done};
use tauri::http::{header, Method, Request, Response, StatusCode};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::host_call;
use crate::state::SharedState;

/// The scheme's name (`rekindle_skin::serve::SCHEME`).
pub const SCHEME: &str = "skin";

/// Answer one request to the scheme.
pub fn handle<R: Runtime>(
    ctx: &UriSchemeContext<'_, R>,
    request: &Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    if request.method() != Method::GET {
        responder.respond(refusal(StatusCode::METHOD_NOT_ALLOWED));
        return;
    }
    let Some(state) = ctx
        .app_handle()
        .try_state::<SharedState>()
        .map(|s| Arc::clone(&s))
    else {
        responder.respond(refusal(StatusCode::SERVICE_UNAVAILABLE));
        return;
    };
    let path = request.uri().path().to_owned();
    tauri::async_runtime::spawn(async move {
        let answer = host_call::call(&state, IpcRequest::SkinFile { path }).await;
        responder.respond(response(answer));
    });
}

/// An empty response with `status`, never cached.
fn refusal(status: StatusCode) -> Response<Vec<u8>> {
    let mut r = Response::new(Vec::new());
    *r.status_mut() = status;
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    r
}

/// The HTTP response for the Host's answer: its bytes with its media type
/// and cache policy, or the Host's refusal code.
#[must_use]
pub fn response(answer: Result<Done<SkinServed>, CommandError>) -> Response<Vec<u8>> {
    let served = match answer {
        Ok(done) => done.value,
        Err(CommandError::Failed { code, .. }) => {
            let status = u16::try_from(code)
                .ok()
                .and_then(|c| StatusCode::from_u16(c).ok())
                .filter(|s| s.is_client_error() || s.is_server_error())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            return refusal(status);
        }
        Err(_) => return refusal(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let built = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, served.content_type)
        .header(header::CACHE_CONTROL, served.cache_control)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(served.bytes);
    built.unwrap_or_else(|_| refusal(StatusCode::INTERNAL_SERVER_ERROR))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn served(cache: &str) -> Done<SkinServed> {
        Done {
            value: SkinServed {
                content_type: "text/css; charset=utf-8".into(),
                cache_control: cache.into(),
                bytes: b"@layer skin {}".to_vec(),
            },
            undo: None,
        }
    }

    fn header_of(r: &Response<Vec<u8>>, name: header::HeaderName) -> &str {
        r.headers().get(name).unwrap().to_str().unwrap()
    }

    #[test]
    fn the_hosts_headers_go_out_unchanged() {
        for cache in ["public, max-age=31536000, immutable", "no-store"] {
            let r = response(Ok(served(cache)));
            assert_eq!(r.status(), StatusCode::OK);
            assert_eq!(header_of(&r, header::CACHE_CONTROL), cache);
            assert_eq!(
                header_of(&r, header::CONTENT_TYPE),
                "text/css; charset=utf-8"
            );
            assert_eq!(header_of(&r, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
            assert_eq!(r.body(), b"@layer skin {}");
        }
    }

    #[test]
    fn an_unknown_address_or_a_bad_path_is_the_hosts_refusal() {
        // The Host answers 404 for an unknown package, an unknown image
        // and any path outside `/{package}/{file}` (`..` included).
        let r = response(Err(CommandError::Failed {
            code: 404,
            message: "no such skin file".into(),
            remediation: None,
        }));
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
        assert_eq!(header_of(&r, header::CACHE_CONTROL), "no-store");
        assert!(r.body().is_empty());
        let odd = response(Err(CommandError::Failed {
            code: 200,
            message: String::new(),
            remediation: None,
        }));
        assert_eq!(odd.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            response(Err(CommandError::Cancelled)).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
