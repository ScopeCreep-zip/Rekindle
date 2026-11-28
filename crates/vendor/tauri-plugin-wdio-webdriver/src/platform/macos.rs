use std::ptr::NonNull;
use std::sync::Arc;

use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{msg_send, MainThreadMarker};
use objc2_app_kit::NSImage;
use objc2_foundation::{
    NSData, NSDictionary, NSError, NSRunLoop, NSRunLoopCommonModes, NSString, NSTimer,
};
use objc2_web_kit::{
    WKContentWorld, WKPDFConfiguration, WKScriptMessageHandler, WKSnapshotConfiguration,
    WKUIDelegate, WKWebView,
};
use serde_json::Value;
use tauri::{Manager, Runtime, WebviewWindow};
use tokio::sync::oneshot;

use crate::eval_channel::EvalResultRegistry;
use crate::platform::alert_state::AlertStateManager;
use crate::platform::executor_util::within_script_timeout;
use crate::platform::macos_convert::{
    self, describe_ns_error, image_to_png_base64, ns_object_to_json,
};
use crate::platform::macos_delegate::{EvalMessageHandler, WebDriverUIDelegate};
use crate::platform::{wrap_script_for_frame_context, FrameId, PlatformExecutor, PrintOptions};
use crate::server::response::WebDriverErrorResponse;
use crate::webdriver::Timeouts;

/// Key for associating the UI delegate with the webview
static DELEGATE_KEY: u8 = 0;

/// macOS `WebView` executor using `WKWebView` native APIs
#[derive(Clone)]
pub struct MacOSExecutor<R: Runtime> {
    window: WebviewWindow<R>,
    timeouts: Timeouts,
    frame_context: Vec<FrameId>,
}

impl<R: Runtime> MacOSExecutor<R> {
    pub fn new(window: WebviewWindow<R>, timeouts: Timeouts, frame_context: Vec<FrameId>) -> Self {
        Self {
            window,
            timeouts,
            frame_context,
        }
    }

    /// Dispatch one `callAsyncJavaScript` attempt of a DirectEval wrapper. The returned receiver fires
    /// when macOS 26.4's WebKit reclaims the completion with no value — the signal `execute_direct_eval`
    /// uses to re-dispatch. When the completion *does* carry a value, it is delivered to `registry` as a
    /// fallback for a lost message-handler post (first-write-wins keeps the post authoritative).
    fn dispatch_eval_attempt(
        &self,
        wrapper: &str,
        id: &str,
        registry: &Arc<EvalResultRegistry>,
        attempt: usize,
    ) -> Result<oneshot::Receiver<()>, String> {
        let (fail_tx, fail_rx) = oneshot::channel::<()>();
        let wrapper = wrapper.to_owned();
        let id = id.to_owned();
        let registry = registry.clone();
        // `RcBlock` is `Fn` but `Sender::send` consumes self, so guard the one-shot send (WebKit calls
        // the completion once; this just makes a double-call harmless). Same pattern as `evaluate_js`.
        let fail_tx = Arc::new(parking_lot::Mutex::new(Some(fail_tx)));

        self.window
            // SAFETY: `with_webview` runs this on the main thread with the live WKWebView;
            // `inner()` is that WKWebView, and the completion blocks below only borrow the
            // non-null pointers WebKit passes them for the duration of the call.
            .with_webview(move |webview| unsafe {
                let wk_webview: &WKWebView = &*webview.inner().cast();
                let ns_script = NSString::from_str(&wrapper);
                let mtm = MainThreadMarker::new_unchecked();
                let empty_dict: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::new();
                let content_world = WKContentWorld::pageWorld(mtm);

                let block = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
                    if !error.is_null() {
                        tracing::debug!(
                            "DirectEval completion reclaimed (attempt {attempt}): {}",
                            describe_ns_error(&*error)
                        );
                        if let Some(tx) = fail_tx.lock().take() {
                            let _ = tx.send(());
                        }
                        return;
                    }
                    if !result.is_null() {
                        registry.complete(&id, ns_object_to_json(&*result));
                    }
                });

                wk_webview.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
                    &ns_script,
                    Some(&empty_dict),
                    None,
                    &content_world,
                    Some(&block),
                );
            })
            .map_err(|e| e.to_string())?;

        Ok(fail_rx)
    }
}

/// Register `WKWebView` handlers at webview creation time.
/// This is called from the plugin's `on_webview_ready` hook to ensure
/// the UI delegate is registered before any navigation completes.
pub fn register_webview_handlers<R: Runtime>(webview: &tauri::Webview<R>) {
    use objc2::ffi::{objc_setAssociatedObject, OBJC_ASSOCIATION_RETAIN_NONATOMIC};

    // Get per-window alert state from the manager
    let manager = webview.app_handle().state::<AlertStateManager>();
    let alert_state = manager.get_or_create(webview.label());
    // Cloned Arc so the (non-generic) objc2 message handler can hold it for the webview's lifetime.
    let eval_registry = webview
        .app_handle()
        .state::<Arc<EvalResultRegistry>>()
        .inner()
        .clone();

    // SAFETY: `with_webview` runs this on the main thread with the live WKWebView;
    // `inner()` is that WKWebView, and the completion blocks below only borrow the
    // non-null pointers WebKit passes them for the duration of the call.
    let _ = webview.with_webview(move |webview| unsafe {
        let wk_webview: &WKWebView = &*webview.inner().cast();
        let mtm = MainThreadMarker::new_unchecked();

        // Rekindle patch: keep the delegate this one replaces (wry's `WryWebViewUIDelegate`,
        // wry-0.57.0 src/wkwebview/mod.rs:599-605) so media-capture permission and the file
        // open panel keep wry's handling; see `macos_delegate`.
        let previous = wk_webview.UIDelegate();
        let delegate = WebDriverUIDelegate::new(mtm, alert_state, previous);
        let delegate_protocol: Retained<ProtocolObject<dyn WKUIDelegate>> =
            ProtocolObject::from_retained(delegate);

        let _: () = msg_send![wk_webview, setUIDelegate: &*delegate_protocol];

        // Associate delegate with webview - released when webview is deallocated
        objc_setAssociatedObject(
            std::ptr::from_ref::<WKWebView>(wk_webview)
                .cast_mut()
                .cast(),
            std::ptr::addr_of!(DELEGATE_KEY).cast(),
            Retained::into_raw(delegate_protocol).cast(),
            OBJC_ASSOCIATION_RETAIN_NONATOMIC,
        );

        // The DirectEval result channel (see execute_direct_eval). WKUserContentController retains the
        // handler for the webview's lifetime, so — unlike the UI delegate — it needs no associated object.
        let eval_handler = EvalMessageHandler::new(mtm, eval_registry);
        let eval_proto: Retained<ProtocolObject<dyn WKScriptMessageHandler>> =
            ProtocolObject::from_retained(eval_handler);
        wk_webview
            .configuration()
            .userContentController()
            .addScriptMessageHandler_name(&eval_proto, &NSString::from_str("wdioEvalResult"));

        start_runloop_pump();

        tracing::debug!("Registered UI delegate + eval message handler for webview");
    });
}

/// Keep the app's main run loop waking on a headless/off-screen runner.
///
/// With no display, the app parks in `-[NSApplication run]` waiting for events, so WebKit doesn't
/// promptly service WebContent's URL-scheme resource requests (page-load subresources) or eval
/// dispatches — a DirectEval can then stall for tens of seconds and time out. A no-op timer in the
/// run loop's common modes forces it to wake, draining that pending work each tick; the interval only
/// bounds worst-case servicing latency (well under the command timeout). Harmless with a real display
/// (the loop already pumps continuously). This plugin ships only in WebDriver-automation builds.
///
/// `Once`-guarded, so it is safe to schedule from both plugin `setup` (earliest — covers cold-start
/// and deeplink navigation before the first webview is ready) and `on_webview_ready` (fallback).
/// https://github.com/webdriverio/desktop-mobile/issues/540
///
/// # Safety
/// Must be called on the main thread (schedules on the main run loop).
pub unsafe fn start_runloop_pump() {
    use std::sync::Once;
    static PUMP: Once = Once::new();
    PUMP.call_once(|| {
        let block = RcBlock::new(|_timer: NonNull<NSTimer>| {});
        // SAFETY: the caller guarantees the main thread; the timer and block are retained by
        // the run loop, and `NSRunLoopCommonModes` is a valid, immutable Foundation constant.
        unsafe {
            let timer = NSTimer::timerWithTimeInterval_repeats_block(0.05, true, &block);
            NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
        }
        tracing::debug!("Started macOS main run-loop pump for headless WebKit scheduling");
    });
}

#[async_trait]
impl<R: Runtime + 'static> PlatformExecutor<R> for MacOSExecutor<R> {
    // =========================================================================
    // Window Access
    // =========================================================================

    fn window(&self) -> &WebviewWindow<R> {
        &self.window
    }

    fn script_timeout_ms(&self) -> u64 {
        self.timeouts.script
    }

    async fn dispatch_native_key(
        &self,
        key: super::NativeKey,
        is_down: bool,
        modifiers: super::ModifierState,
    ) -> Result<(), WebDriverErrorResponse> {
        super::macos_keys::dispatch_to_window(
            &self.window,
            key,
            is_down,
            modifiers,
            self.timeouts.script,
        )
        .await
    }

    // Core JavaScript execution.

    async fn evaluate_js(&self, script: &str) -> Result<Value, WebDriverErrorResponse> {
        let (tx, rx) = oneshot::channel();
        let script_owned = wrap_script_for_frame_context(script, &self.frame_context);

        // SAFETY: `with_webview` runs this on the main thread with the live WKWebView;
        // `inner()` is that WKWebView, and the completion blocks below only borrow the
        // non-null pointers WebKit passes them for the duration of the call.
        let result = self.window.with_webview(move |webview| unsafe {
            let wk_webview: &WKWebView = &*webview.inner().cast();
            let ns_script = NSString::from_str(&script_owned);

            let tx = Arc::new(parking_lot::Mutex::new(Some(tx)));
            let block = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
                let response = if !error.is_null() {
                    let error_ref = &*error;
                    Err(describe_ns_error(error_ref))
                } else if result.is_null() {
                    Ok(Value::Null)
                } else {
                    let obj = &*result;
                    Ok(ns_object_to_json(obj))
                };

                if let Some(tx) = tx.lock().take() {
                    let _ = tx.send(response);
                }
            });

            // Use evaluateJavaScript for script execution
            wk_webview.evaluateJavaScript_completionHandler(&ns_script, Some(&block));
        });

        if let Err(e) = result {
            return Err(WebDriverErrorResponse::javascript_error(
                &e.to_string(),
                None,
            ));
        }

        let timeout = std::time::Duration::from_millis(self.timeouts.script);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(value))) => Ok(serde_json::json!({
                "success": true,
                "value": value
            })),
            Ok(Ok(Err(error))) => Err(WebDriverErrorResponse::javascript_error(&error, None)),
            Ok(Err(_)) => Err(WebDriverErrorResponse::unknown_error("Channel closed")),
            Err(_) => Err(WebDriverErrorResponse::script_timeout()),
        }
    }

    // =========================================================================
    // Async Script Execution (using callAsyncJavaScript)
    // =========================================================================

    async fn execute_async_script(
        &self,
        script: &str,
        args: &[Value],
    ) -> Result<Value, WebDriverErrorResponse> {
        let args_json = serde_json::to_string(args)
            .map_err(|e| WebDriverErrorResponse::invalid_argument(&e.to_string()))?;

        // Build wrapper that includes argument deserialization.
        // callAsyncJavaScript treats the body as function statements, so `return` is required —
        // without it the function returns undefined immediately and the Promise is discarded.
        let wrapper = macos_convert::async_script_wrapper(script, &args_json);

        let (tx, rx) = oneshot::channel();

        // SAFETY: `with_webview` runs this on the main thread with the live WKWebView;
        // `inner()` is that WKWebView, and the completion blocks below only borrow the
        // non-null pointers WebKit passes them for the duration of the call.
        let result = self.window.with_webview(move |webview| unsafe {
            let wk_webview: &WKWebView = &*webview.inner().cast();
            let ns_script = NSString::from_str(&wrapper);
            let mtm = MainThreadMarker::new_unchecked();

            // Empty dictionary for arguments (we pass args via JSON in the script)
            let empty_dict: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::new();

            // Get the page content world
            let content_world = WKContentWorld::pageWorld(mtm);

            let tx = Arc::new(parking_lot::Mutex::new(Some(tx)));
            let block = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
                let response = if !error.is_null() {
                    let error_ref = &*error;
                    Err(describe_ns_error(error_ref))
                } else if result.is_null() {
                    Ok(Value::Null)
                } else {
                    let obj = &*result;
                    Ok(ns_object_to_json(obj))
                };

                if let Some(tx) = tx.lock().take() {
                    let _ = tx.send(response);
                }
            });

            wk_webview.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
                &ns_script,
                Some(&empty_dict),
                None,
                &content_world,
                Some(&block),
            );
        });

        if let Err(e) = result {
            return Err(WebDriverErrorResponse::javascript_error(
                &e.to_string(),
                None,
            ));
        }

        let timeout = std::time::Duration::from_millis(self.timeouts.script);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(error))) => Err(WebDriverErrorResponse::javascript_error(&error, None)),
            Ok(Err(_)) => Err(WebDriverErrorResponse::unknown_error("Channel closed")),
            Err(_) => Err(WebDriverErrorResponse::script_timeout()),
        }
    }

    /// DirectEval (`/wdio/eval`) with out-of-band result delivery + reclaim recovery.
    ///
    /// Each attempt is dispatched by `dispatch_eval_attempt` via `callAsyncJavaScript`, which keeps the
    /// run loop pumping — on a headless runner the app's own `core.invoke` IPC response only completes
    /// while WebKit holds a run-loop activity. The result is delivered primarily out-of-band: the
    /// wrapper posts `{ id, result }` to the `wdioEvalResult` `WKScriptMessageHandler`, which completes
    /// the `oneshot` awaited here. macOS 26.4's WebKit intermittently reclaims the `callAsyncJavaScript`
    /// completion ("Completion handler for function call is no longer reachable"); usually the post
    /// still lands, but sometimes the script never posts and this would otherwise dead-wait to the
    /// script timeout (#540 residual). Two guards recover it without changing the happy path:
    ///   1. **Fallback delivery** — when the completion *does* fire with a value, `dispatch_eval_attempt`
    ///      hands it to the same registry (first-write-wins keeps the post authoritative), so a lost
    ///      post resolves instead of dead-waiting.
    ///   2. **Bounded re-dispatch** — when the completion is reclaimed with *no* value and no post lands
    ///      within `RECLAIM_GRACE`, re-run the script (up to `MAX_ATTEMPTS`). The grace lets a racing
    ///      post settle first; a genuine re-dispatch *re-executes* the script, so a non-idempotent eval
    ///      could double-apply — acceptable for the read-style evals `execute` runs in practice.
    /// The whole sequence is bounded by `deadline`. Mirrors the Windows native-handler path
    /// (`AsyncScriptState`). https://github.com/webdriverio/desktop-mobile/issues/540
    ///
    /// `wrapScriptForDirectEval` produces a callback-style script (`arguments[last]` is its
    /// done-callback); `__report` forwards the result to the message handler then `resolve`s so the
    /// `callAsyncJavaScript` promise settles.
    async fn execute_direct_eval(&self, script: &str) -> Result<Value, WebDriverErrorResponse> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = format!("wdio_de_{}", COUNTER.fetch_add(1, Ordering::Relaxed));

        // Register the waiter before dispatching, so a fast report can't arrive before we're listening.
        let registry = self
            .window
            .state::<Arc<EvalResultRegistry>>()
            .inner()
            .clone();
        // Released however this call ends, including a drop mid-await.
        let (_registration, mut rx) = registry.register_pending(id.clone());

        // id is alphanumeric + '_', so it needs no escaping in the JS string literal below.
        let wrapper = format!(
            r#"return new Promise((resolve) => {{
  var __report = function (r) {{
    try {{ window.webkit.messageHandlers.wdioEvalResult.postMessage({{ id: "{id}", result: r }}); }} catch (e) {{}}
    resolve(r);
  }};
  (function () {{ {script} }}).apply(null, [__report]);
}});"#
        );

        const MAX_ATTEMPTS: usize = 3;
        // After a reclaim, wait this long for a racing message-handler post before re-running.
        const RECLAIM_GRACE: std::time::Duration = std::time::Duration::from_millis(750);
        let channel_closed = || WebDriverErrorResponse::unknown_error("Eval result channel closed");

        // Retry only when WebKit reclaims the completion before a result was delivered; the
        // session's script timeout bounds the whole sequence (`within_script_timeout`, the one bound
        // every execute path goes through), so its expiry is the single script-timeout path.
        within_script_timeout(self.script_timeout_ms(), async {
            for attempt in 0..MAX_ATTEMPTS {
                let fail_rx = self
                    .dispatch_eval_attempt(&wrapper, &id, &registry, attempt)
                    .map_err(|e| WebDriverErrorResponse::javascript_error(&e, None))?;
                tokio::select! {
                    r = &mut rx => return r.map_err(|_| channel_closed()),
                    _ = fail_rx => {
                        if attempt + 1 == MAX_ATTEMPTS {
                            // Last attempt reclaimed: keep waiting for a late post until the deadline.
                            return (&mut rx).await.map_err(|_| channel_closed());
                        }
                        // Race a late post against the grace: if the script actually ran and posts
                        // here, return it — re-dispatching would execute the script (and any mocks it
                        // hits) a second time. Only re-dispatch once the grace elapses with no result.
                        tokio::select! {
                            r = &mut rx => return r.map_err(|_| channel_closed()),
                            () = tokio::time::sleep(RECLAIM_GRACE) => {}
                        }
                    }
                }
            }
            Err(WebDriverErrorResponse::script_timeout()) // unreachable: the final attempt returns above
        })
        .await
    }

    // =========================================================================
    // Screenshots
    // =========================================================================

    async fn take_screenshot(&self) -> Result<String, WebDriverErrorResponse> {
        let (tx, rx) = oneshot::channel();

        // SAFETY: `with_webview` runs this on the main thread with the live WKWebView;
        // `inner()` is that WKWebView, and the completion blocks below only borrow the
        // non-null pointers WebKit passes them for the duration of the call.
        let result = self.window.with_webview(move |webview| unsafe {
            let wk_webview: &WKWebView = &*webview.inner().cast();
            let mtm = MainThreadMarker::new_unchecked();
            let config = WKSnapshotConfiguration::new(mtm);

            let tx = Arc::new(parking_lot::Mutex::new(Some(tx)));
            let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
                let response = if !error.is_null() {
                    let error_ref = &*error;
                    Err(describe_ns_error(error_ref))
                } else if image.is_null() {
                    Err("No image returned".to_string())
                } else {
                    let image_ref = &*image;
                    image_to_png_base64(image_ref)
                };

                if let Some(tx) = tx.lock().take() {
                    let _ = tx.send(response);
                }
            });

            wk_webview.takeSnapshotWithConfiguration_completionHandler(Some(&config), &block);
        });

        if let Err(e) = result {
            return Err(WebDriverErrorResponse::unknown_error(&e.to_string()));
        }

        let timeout = std::time::Duration::from_millis(self.timeouts.script);
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(base64))) => Ok(base64),
            Ok(Ok(Err(error))) => Err(WebDriverErrorResponse::unknown_error(&error)),
            Ok(Err(_)) => Err(WebDriverErrorResponse::unknown_error("Channel closed")),
            Err(_) => Err(WebDriverErrorResponse::script_timeout()),
        }
    }

    async fn take_element_screenshot(
        &self,
        js_var: &str,
    ) -> Result<String, WebDriverErrorResponse> {
        // For element screenshots, we use JavaScript canvas approach
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}

                // Use html2canvas-like approach if available, otherwise scroll into view
                el.scrollIntoView({{ block: 'center', inline: 'center' }});

                // Return element bounds for clipping
                var rect = el.getBoundingClientRect();
                return {{
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height
                }};
            }})()"
        );
        self.evaluate_js(&script).await?;

        self.take_screenshot().await
    }

    // =========================================================================
    // Print
    // =========================================================================

    async fn print_page(&self, options: PrintOptions) -> Result<String, WebDriverErrorResponse> {
        // First, inject CSS @page rules for print settings
        let css_script = macos_convert::print_style_script(&options);
        self.evaluate_js(&css_script).await?;

        // Now create PDF using WKWebView's native API
        let (tx, rx) = oneshot::channel();

        // SAFETY: `with_webview` runs this on the main thread with the live WKWebView;
        // `inner()` is that WKWebView, and the completion blocks below only borrow the
        // non-null pointers WebKit passes them for the duration of the call.
        let result = self.window.with_webview(move |webview| unsafe {
            let wk_webview: &WKWebView = &*webview.inner().cast();
            let mtm = MainThreadMarker::new_unchecked();

            // Create PDF configuration
            let config = WKPDFConfiguration::new(mtm);
            // Note: WKPDFConfiguration only has rect and allowTransparentBackground
            // Page size/margins are handled via CSS @page rules above

            let tx = Arc::new(parking_lot::Mutex::new(Some(tx)));
            let block = RcBlock::new(move |data: *mut NSData, error: *mut NSError| {
                let response = if !error.is_null() {
                    let error_ref = &*error;
                    Err(describe_ns_error(error_ref))
                } else if data.is_null() {
                    Err("No PDF data returned".to_string())
                } else {
                    let data_ref = &*data;
                    let bytes = data_ref.to_vec();
                    Ok(bytes)
                };

                if let Some(tx) = tx.lock().take() {
                    let _ = tx.send(response);
                }
            });

            wk_webview.createPDFWithConfiguration_completionHandler(Some(&config), &block);
        });

        if let Err(e) = result {
            return Err(WebDriverErrorResponse::unknown_error(&e.to_string()));
        }

        // Wait for result
        let timeout = std::time::Duration::from_millis(self.timeouts.script);
        let pdf_result = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(bytes))) => Ok(bytes),
            Ok(Ok(Err(error))) => Err(WebDriverErrorResponse::unknown_error(&error)),
            Ok(Err(_)) => Err(WebDriverErrorResponse::unknown_error("Channel closed")),
            Err(_) => Err(WebDriverErrorResponse::script_timeout()),
        };

        // Clean up injected style
        let _ = self
            .evaluate_js(
                r"(function() {
                var style = document.getElementById('__webdriver_print_style');
                if (style) style.remove();
                return true;
            })()",
            )
            .await;

        // Return base64 encoded PDF
        pdf_result.map(|bytes| BASE64_STANDARD.encode(&bytes))
    }
}
