//! Request guard (Rekindle patch 1; plan E7.13 "Driver", SEC1).
//!
//! Upstream 1.5.1 binds `127.0.0.1` with no token, `Host` or `Origin` check
//! anywhere in `src/server` and serves sessionless `POST /wdio/eval`, so any
//! local process, or a web page reaching the port by DNS rebinding, could
//! script every window. This guard follows the E2E bridge's rules
//! (`crates/rekindle-e2e-server/src/lib.rs`, `RequestGuard::admit`) and wraps every
//! route, the fallback included (`guarded`). A request is refused when it:
//! - names a `Host` other than `127.0.0.1:<port>` (`FORBIDDEN`);
//! - carries any `Origin`: the WebDriver client is a test process, which
//!   sends none, while a browser always sends one on a cross-origin request
//!   (`FORBIDDEN`);
//! - lacks `Authorization: Bearer <token>` with the run's token, compared in
//!   constant time by `rekindle_secrets::bearer_token::BearerToken::matches`
//!   (`UNAUTHORIZED`).

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use rekindle_secrets::bearer_token::BearerToken;

/// The variable the walk's runner sets to the run's token
/// (`BearerToken::mint`'s spelling).
pub const TOKEN_ENV_VAR: &str = "REKINDLE_WDIO_TOKEN";

const BEARER_PREFIX: &str = "Bearer ";

/// What a request must match to reach the driver.
pub struct RequestGuard {
    host: String,
    token: BearerToken,
}

impl RequestGuard {
    /// The guard for a server bound to `127.0.0.1:port`.
    #[must_use]
    pub fn new(port: u16, token: BearerToken) -> Self {
        Self {
            host: format!("127.0.0.1:{port}"),
            token,
        }
    }

    /// The guard for this run: the token from [`TOKEN_ENV_VAR`], or `None`
    /// when it is unset or is not a minted token's spelling.
    #[must_use]
    pub fn from_env(port: u16) -> Option<Self> {
        let spelled = std::env::var(TOKEN_ENV_VAR).ok()?;
        BearerToken::parse(&spelled).map(|token| Self::new(port, token))
    }

    /// `Ok`, or the status a request with `headers` is refused with.
    ///
    /// # Errors
    /// `FORBIDDEN` for a foreign `Host` or any `Origin`, `UNAUTHORIZED` for
    /// a missing or wrong token.
    pub fn admit(&self, headers: &HeaderMap) -> Result<(), StatusCode> {
        let host = headers
            .get(header::HOST)
            .map(axum::http::HeaderValue::as_bytes);
        if host != Some(self.host.as_bytes()) {
            return Err(StatusCode::FORBIDDEN);
        }
        if headers.contains_key(header::ORIGIN) {
            return Err(StatusCode::FORBIDDEN);
        }
        let presented = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.as_bytes().strip_prefix(BEARER_PREFIX.as_bytes()));
        match presented {
            Some(token) if self.token.matches(token) => Ok(()),
            _ => Err(StatusCode::UNAUTHORIZED),
        }
    }
}

async fn admit_request(
    State(guard): State<Arc<RequestGuard>>,
    request: Request,
    next: Next,
) -> Response {
    match guard.admit(request.headers()) {
        Ok(()) => next.run(request).await,
        Err(status) => status.into_response(),
    }
}

/// `router` with every route and its fallback behind `guard`.
pub fn guarded(router: Router, guard: Arc<RequestGuard>) -> Router {
    router.layer(middleware::from_fn_with_state(guard, admit_request))
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
