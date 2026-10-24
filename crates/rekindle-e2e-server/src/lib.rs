//! The E2E bridge's request guard (research batch 8 AL, F1).
//!
//! The bridge drives a real Host over HTTP, so while it runs anything that
//! can reach its port could drive the Host too, including a webpage in the
//! user's browser. It binds a random loopback port, mints a per-run token
//! and writes both to the owner-only handshake file the Playwright setup
//! names (`e2e/fixtures/e2e-server-setup.ts`), never to a log,
//! and [`Guard::admit`] refuses every request that:
//! - names a `Host` other than the loopback address it bound (DNS
//!   rebinding: a page on a hostile name that resolves to 127.0.0.1);
//! - carries an `Origin` other than the frontend's (the Vite dev server);
//! - lacks the token in [`TOKEN_HEADER`] (compared in constant time).
//!
//! The bridge is a test tool: it does not compile in a release build (the
//! `compile_error!` below; `cargo xtask check-e2e-server-release`).

#[cfg(not(debug_assertions))]
compile_error!("rekindle-e2e-server is a test bridge and never builds in release");

use axum::http::{header, HeaderMap, StatusCode};
use rekindle_secrets::bearer_token::BearerToken;

/// The header the token travels in.
pub const TOKEN_HEADER: &str = "x-rekindle-e2e-token";

/// The variable naming the file the bridge writes its address and token
/// to: in a directory the Playwright setup created for this run, written
/// owner-only (0600) and moved into place whole, so the token never reaches
/// stdout or a CI log.
pub const HANDSHAKE_ENV: &str = "REKINDLE_E2E_HANDSHAKE";

/// Write `contents` to `path` readable by its owner only: a sibling file
/// created new with mode 0600, then renamed over `path` so a reader never
/// sees a partial write.
///
/// # Errors
/// The sibling file exists or cannot be written, or the rename fails.
pub fn write_handshake(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let partial = path.with_extension("partial");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&partial)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&partial, path)
}

/// What a request must match to reach the bridge.
pub struct Guard {
    hosts: [String; 2],
    origin: String,
    token: BearerToken,
}

impl Guard {
    /// The guard for a bridge bound to loopback `port`, answering the
    /// browser origin `origin`.
    #[must_use]
    pub fn new(port: u16, origin: String, token: BearerToken) -> Self {
        Self {
            hosts: [format!("127.0.0.1:{port}"), format!("localhost:{port}")],
            origin,
            token,
        }
    }

    /// The browser origin the bridge answers.
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// The run's token.
    #[must_use]
    pub fn token(&self) -> &BearerToken {
        &self.token
    }

    /// Whether a request with `headers` may reach the bridge: `Ok`, or the
    /// status it is refused with.
    ///
    /// # Errors
    ///
    /// `FORBIDDEN` for a foreign `Host` or `Origin`, `UNAUTHORIZED` for a
    /// missing or wrong token.
    pub fn admit(&self, headers: &HeaderMap) -> Result<(), StatusCode> {
        let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
        if !host.is_some_and(|h| self.hosts.iter().any(|ours| ours == h)) {
            return Err(StatusCode::FORBIDDEN);
        }
        // A request from the test process carries no Origin; a browser
        // always sends one on a cross-origin request.
        if let Some(origin) = headers.get(header::ORIGIN) {
            if origin.as_bytes() != self.origin.as_bytes() {
                return Err(StatusCode::FORBIDDEN);
            }
        }
        match headers.get(TOKEN_HEADER) {
            Some(token) if self.token.matches(token.as_bytes()) => Ok(()),
            _ => Err(StatusCode::UNAUTHORIZED),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Guard, TOKEN_HEADER};
    use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
    use rekindle_secrets::bearer_token::BearerToken;

    const ORIGIN: &str = "http://localhost:1430";

    fn guard() -> (Guard, String) {
        let token = BearerToken::mint();
        let spelled = token.as_str().to_owned();
        (Guard::new(4242, ORIGIN.into(), token), spelled)
    }

    fn headers(host: &str, origin: Option<&str>, token: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_str(host).unwrap());
        if let Some(o) = origin {
            h.insert(header::ORIGIN, HeaderValue::from_str(o).unwrap());
        }
        if let Some(t) = token {
            h.insert(TOKEN_HEADER, HeaderValue::from_str(t).unwrap());
        }
        h
    }

    #[test]
    fn the_test_process_and_the_frontend_are_admitted() {
        let (g, t) = guard();
        assert_eq!(g.admit(&headers("127.0.0.1:4242", None, Some(&t))), Ok(()));
        assert_eq!(
            g.admit(&headers("localhost:4242", Some(ORIGIN), Some(&t))),
            Ok(())
        );
    }

    #[test]
    fn a_missing_or_wrong_token_is_unauthorized() {
        let (g, t) = guard();
        let unauthorized = Err(StatusCode::UNAUTHORIZED);
        assert_eq!(
            g.admit(&headers("127.0.0.1:4242", None, None)),
            unauthorized
        );
        let wrong = BearerToken::mint();
        assert_eq!(
            g.admit(&headers("127.0.0.1:4242", None, Some(wrong.as_str()))),
            unauthorized
        );
        assert_eq!(
            g.admit(&headers("127.0.0.1:4242", None, Some(&t[1..]))),
            unauthorized
        );
    }

    #[test]
    fn a_foreign_host_or_origin_is_forbidden_even_with_the_token() {
        let (g, t) = guard();
        let forbidden = Err(StatusCode::FORBIDDEN);
        assert_eq!(
            g.admit(&headers("evil.example:4242", None, Some(&t))),
            forbidden
        );
        assert_eq!(
            g.admit(&headers("127.0.0.1:3001", None, Some(&t))),
            forbidden
        );
        assert_eq!(
            g.admit(&headers(
                "127.0.0.1:4242",
                Some("https://evil.example"),
                Some(&t)
            )),
            forbidden
        );
        assert_eq!(
            g.admit(&headers("127.0.0.1:4242", Some("null"), Some(&t))),
            forbidden
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_handshake_file_is_owner_only_and_whole() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("rekindle-e2e-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bridge.json");
        super::write_handshake(&path, "{\"url\":\"u\",\"token\":\"t\"}").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"url\":\"u\",\"token\":\"t\"}"
        );
        assert!(!path.with_extension("partial").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
