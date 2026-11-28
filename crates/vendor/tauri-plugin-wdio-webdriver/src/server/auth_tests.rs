use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use rekindle_secrets::bearer_token::BearerToken;
use tower::ServiceExt as _;

use super::{guarded, RequestGuard};

const PORT: u16 = 4242;

fn router() -> (Router, String) {
    let token = BearerToken::mint();
    let spelled = token.as_str().to_owned();
    let inner = Router::new()
        .route("/status", get(|| async { "ok" }))
        .route("/wdio/eval", post(|| async { "ran" }));
    (
        guarded(inner, Arc::new(RequestGuard::new(PORT, token))),
        spelled,
    )
}

async fn status_of(
    router: Router,
    path: &str,
    host: Option<&str>,
    origin: Option<&str>,
    auth: Option<&str>,
) -> StatusCode {
    let mut request = Request::builder().method("POST").uri(path);
    if path == "/status" {
        request = request.method("GET");
    }
    if let Some(h) = host {
        request = request.header(header::HOST, h);
    }
    if let Some(o) = origin {
        request = request.header(header::ORIGIN, o);
    }
    if let Some(a) = auth {
        request = request.header(header::AUTHORIZATION, a);
    }
    let response = router
        .oneshot(request.body(Body::empty()).expect("request"))
        .await
        .expect("infallible");
    response.status()
}

const HOST: &str = "127.0.0.1:4242";

#[tokio::test]
async fn the_test_process_with_the_token_is_admitted_to_every_route() {
    let (router, token) = router();
    let auth = format!("Bearer {token}");
    for path in ["/status", "/wdio/eval"] {
        let status = status_of(router.clone(), path, Some(HOST), None, Some(&auth)).await;
        assert_eq!(status, StatusCode::OK, "{path}");
    }
}

#[tokio::test]
async fn eval_without_or_with_a_wrong_token_is_unauthorized() {
    let (router, token) = router();
    let wrong = format!("Bearer {}", BearerToken::mint().as_str());
    let bare = token.clone();
    let short = format!("Bearer {}", &token[..51]);
    for auth in [
        None,
        Some(wrong.as_str()),
        Some(bare.as_str()),
        Some(short.as_str()),
    ] {
        let status = status_of(router.clone(), "/wdio/eval", Some(HOST), None, auth).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{auth:?}");
    }
}

#[tokio::test]
async fn a_foreign_or_missing_host_is_forbidden_even_with_the_token() {
    let (router, token) = router();
    let auth = format!("Bearer {token}");
    for host in [
        None,
        Some("localhost:4242"),
        Some("127.0.0.1:4243"),
        Some("evil.example:4242"),
        Some("127.0.0.1"),
    ] {
        let status = status_of(router.clone(), "/wdio/eval", host, None, Some(&auth)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{host:?}");
    }
}

#[tokio::test]
async fn any_origin_is_forbidden_even_with_the_token() {
    let (router, token) = router();
    let auth = format!("Bearer {token}");
    for origin in ["http://127.0.0.1:4242", "https://evil.example", "null"] {
        let status = status_of(
            router.clone(),
            "/wdio/eval",
            Some(HOST),
            Some(origin),
            Some(&auth),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{origin}");
    }
}

#[tokio::test]
async fn an_unknown_route_is_behind_the_guard_too() {
    let (router, _token) = router();
    let status = status_of(router, "/session/x/execute/sync", Some(HOST), None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
