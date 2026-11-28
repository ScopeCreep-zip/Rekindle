//! WebKitGTK's native JavaScript evaluation and its reply deadlines.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use glib::MainContext;
use serde_json::Value;
use tauri::Runtime;
use tokio::sync::oneshot;
use webkit2gtk::WebViewExt;

use super::{js_value_to_json, LinuxExecutor};
use crate::platform::executor_util::within_script_timeout;
use crate::platform::{wrap_script_for_frame_context, PlatformExecutor};
use crate::server::response::WebDriverErrorResponse;

/// Shared diagnostic sequence for synchronous and asynchronous evaluations.
static EVAL_SEQ: AtomicU64 = AtomicU64::new(1);

pub(super) async fn evaluate_js<R: Runtime + 'static>(
    executor: &LinuxExecutor<R>,
    script: &str,
) -> Result<Value, WebDriverErrorResponse> {
    // Each hop of the round trip is logged at debug with this call's id
    // (lead, 2026-11-27: an `execute/sync` with no reply for 90 s while
    // the web process idled could not show where its answer died). The
    // hops: queued for the GTK main thread (`with_webview`), run there,
    // the script handed to the web process, its answer, the answer sent
    // back, and the caller's wait.
    let id = EVAL_SEQ.fetch_add(1, Ordering::Relaxed);
    let label = executor.window.label().to_owned();
    let queued = Instant::now();
    tracing::debug!(id, window = %label, script_len = script.len(), "eval queued for the main thread");
    let (tx, rx) = oneshot::channel();
    let script_owned = wrap_script_for_frame_context(script, &executor.frame_context);

    let result = executor.window.with_webview(move |webview| {
        tracing::debug!(id, waited = ?queued.elapsed(), "eval running on the main thread");
        let webview = webview.inner().clone();
        let tx = Arc::new(parking_lot::Mutex::new(Some(tx)));

        // Use glib main context to spawn the async future
        let ctx = MainContext::default();
        ctx.spawn_local(async move {
            tracing::debug!(id, waited = ?queued.elapsed(), "eval script handed to the web process");
            let result = webview
                .evaluate_javascript_future(&script_owned, None, None)
                .await;
            let response: Result<Value, String> = match result {
                // Try multiple serialization strategies for robustness
                Ok(js_value) => Ok(js_value_to_json(&js_value)),
                Err(e) => Err(e.to_string()),
            };
            match &response {
                Ok(_) => tracing::debug!(id, elapsed = ?queued.elapsed(), "eval answered"),
                Err(error) => tracing::debug!(id, elapsed = ?queued.elapsed(), %error, "eval rejected by the web process"),
            }

            if let Some(tx) = tx.lock().take() {
                if tx.send(response).is_err() {
                    tracing::debug!(id, elapsed = ?queued.elapsed(), "eval answer arrived after its caller stopped waiting");
                }
            }
        });
    });

    if let Err(e) = result {
        tracing::debug!(id, window = %label, error = %e, "eval could not reach the webview");
        return Err(WebDriverErrorResponse::javascript_error(
            &e.to_string(),
            None,
        ));
    }

    let timeout = std::time::Duration::from_millis(executor.timeouts.script);
    match tokio::time::timeout(timeout, rx).await {
        Ok(Ok(Ok(value))) => Ok(serde_json::json!({
            "success": true,
            "value": value
        })),
        Ok(Ok(Err(error))) => Err(WebDriverErrorResponse::javascript_error(&error, None)),
        Ok(Err(_)) => {
            tracing::debug!(id, window = %label, elapsed = ?queued.elapsed(), "eval channel closed without an answer");
            Err(WebDriverErrorResponse::unknown_error("Channel closed"))
        }
        Err(_) => {
            tracing::debug!(id, window = %label, elapsed = ?queued.elapsed(), "eval timed out waiting for its answer");
            Err(WebDriverErrorResponse::script_timeout())
        }
    }
}

/// The script's awaited value straight from WebKitGTK's
/// `webkit_web_view_call_async_javascript_function` (2.40+, our floor, plan E7.10;
/// <https://webkitgtk.org/reference/webkitgtk/stable/method.WebView.call_async_javascript_function.html>):
/// it runs `body` as a function body, awaits a returned promise, and answers with its value
/// or the error that stopped it. A document replaced mid-call fails here at once (its
/// context is gone), so no page global and no poll can be left waiting on it.
pub(super) async fn call_async_function<R: Runtime + 'static>(
    executor: &LinuxExecutor<R>,
    body: &str,
) -> Result<Value, WebDriverErrorResponse> {
    let id = EVAL_SEQ.fetch_add(1, Ordering::Relaxed);
    let label = executor.window.label().to_owned();
    let queued = Instant::now();
    tracing::debug!(id, window = %label, body_len = body.len(), "async call queued for the main thread");
    let (tx, rx) = oneshot::channel();
    // The body becomes an async IIFE expression, so the frame wrapper (which evaluates an
    // expression in the selected frame) can carry it, and its promise is the function's return.
    let expression = format!("(async function () {{ {body} }})()");
    let body_owned = format!(
        "return {};",
        wrap_script_for_frame_context(&expression, &executor.frame_context)
    );

    let result = executor.window.with_webview(move |webview| {
        tracing::debug!(id, waited = ?queued.elapsed(), "async call running on the main thread");
        let webview = webview.inner().clone();
        let tx = Arc::new(parking_lot::Mutex::new(Some(tx)));
        let ctx = MainContext::default();
        ctx.spawn_local(async move {
            let result = webview
                .call_async_javascript_function_future(&body_owned, None, None, None)
                .await;
            let response: Result<Value, String> = match result {
                Ok(js_value) => Ok(js_value_to_json(&js_value)),
                Err(e) => Err(e.to_string()),
            };
            match &response {
                Ok(_) => tracing::debug!(id, elapsed = ?queued.elapsed(), "async call answered"),
                Err(error) => tracing::debug!(id, elapsed = ?queued.elapsed(), %error, "async call failed in the web process"),
            }
            if let Some(tx) = tx.lock().take() {
                if tx.send(response).is_err() {
                    tracing::debug!(id, elapsed = ?queued.elapsed(), "async call answer arrived after its caller stopped waiting");
                }
            }
        });
    });

    if let Err(e) = result {
        tracing::debug!(id, window = %label, error = %e, "async call could not reach the webview");
        return Err(WebDriverErrorResponse::javascript_error(
            &e.to_string(),
            None,
        ));
    }

    let answer = async {
        match rx.await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(WebDriverErrorResponse::javascript_error(&error, None)),
            Err(_) => Err(WebDriverErrorResponse::unknown_error("Channel closed")),
        }
    };
    let result = within_script_timeout(executor.script_timeout_ms(), answer).await;
    if matches!(&result, Err(e) if e.error == "script timeout") {
        tracing::debug!(id, window = %label, elapsed = ?queued.elapsed(), "async call timed out waiting for its answer");
    }
    result
}
