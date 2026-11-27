//! Every WebDriver request's arrival and completion at debug, under one id
//! (lead, 2026-11-27: an `execute/sync` whose eval was never queued could
//! not show whether the request reached the plugin, waited inside it, or
//! never arrived). Layered outside the auth guard, so a request that waits
//! anywhere in the plugin logs its arrival and no completion.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

/// The id each request logs its arrival and completion under.
static REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);

/// Log `request`'s arrival, run it, and log its status and elapsed time.
pub async fn log_request(request: Request, next: Next) -> Response {
    let id = REQUEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let started = Instant::now();
    tracing::debug!(id, %method, %path, "webdriver request arrived");
    let response = next.run(request).await;
    tracing::debug!(id, %method, %path, status = response.status().as_u16(), elapsed = ?started.elapsed(), "webdriver request done");
    response
}
