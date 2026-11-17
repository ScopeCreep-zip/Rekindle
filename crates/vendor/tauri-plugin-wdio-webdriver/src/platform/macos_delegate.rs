//! The plugin's `WKUIDelegate` (JavaScript alert/confirm/prompt capture) and the
//! DirectEval script-message handler.
//!
//! Rekindle patch: upstream's delegate replaced wry's and dropped wry's media-capture
//! permission and file open-panel handling. This one keeps the delegate it replaced and
//! forwards those two selectors to it, so camera, microphone and file pickers behave as
//! they do in the shipped app.

use std::sync::Arc;

use block2::DynBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSString, NSURL};
use objc2_web_kit::{
    WKFrameInfo, WKMediaCaptureType, WKOpenPanelParameters, WKPermissionDecision, WKScriptMessage,
    WKScriptMessageHandler, WKSecurityOrigin, WKUIDelegate, WKUserContentController, WKWebView,
};
use serde_json::Value;

use crate::eval_channel::EvalResultRegistry;
use crate::platform::alert_state::{AlertResponse, AlertState, AlertType, PendingAlert};
use crate::platform::macos_convert::ns_object_to_json;

/// How long a JavaScript dialog waits for a WebDriver accept/dismiss.
const DIALOG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Instance variables for the UI delegate.
pub(super) struct WebDriverUIDelegateIvars {
    /// Per-window alert state.
    alert_state: Arc<AlertState>,
    /// The UI delegate this one replaced (wry's), retained so it outlives the swap.
    previous: Option<Retained<ProtocolObject<dyn WKUIDelegate>>>,
}

impl WebDriverUIDelegateIvars {
    /// Publish a dialog to WebDriver and wait for its response (or the timeout).
    fn await_dialog(
        &self,
        message: String,
        default_text: Option<String>,
        alert_type: AlertType,
    ) -> Option<AlertResponse> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.alert_state.set_pending(PendingAlert {
            message,
            default_text,
            alert_type,
            responder: tx,
        });
        rx.recv_timeout(DIALOG_TIMEOUT).ok()
    }

    /// The replaced delegate, when it implements `selector`.
    fn previous_responding_to(
        &self,
        selector: objc2::runtime::Sel,
    ) -> Option<&ProtocolObject<dyn WKUIDelegate>> {
        self.previous
            .as_deref()
            .filter(|previous| previous.respondsToSelector(selector))
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WebDriverUIDelegate"]
    #[ivars = WebDriverUIDelegateIvars]
    pub(super) struct WebDriverUIDelegate;

    unsafe impl NSObjectProtocol for WebDriverUIDelegate {}

    unsafe impl WKUIDelegate for WebDriverUIDelegate {
        /// Handle JavaScript `alert()` calls
        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn run_alert_panel(
            &self,
            _webview: &WKWebView,
            message: &NSString,
            _frame: &WKFrameInfo,
            completion_handler: &DynBlock<dyn Fn()>,
        ) {
            let message_str = message.to_string();
            tracing::debug!("Intercepted alert: {message_str}");
            let _ = self
                .ivars()
                .await_dialog(message_str, None, AlertType::Alert);
            completion_handler.call(());
        }

        /// Handle JavaScript `confirm()` calls
        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn run_confirm_panel(
            &self,
            _webview: &WKWebView,
            message: &NSString,
            _frame: &WKFrameInfo,
            completion_handler: &DynBlock<dyn Fn(Bool)>,
        ) {
            let message_str = message.to_string();
            tracing::debug!("Intercepted confirm: {message_str}");
            let response = self
                .ivars()
                .await_dialog(message_str, None, AlertType::Confirm);
            // True if accepted or timed out, false if dismissed (upstream behaviour).
            let accepted = response.is_none_or(|r| r.accepted);
            completion_handler.call((Bool::from(accepted),));
        }

        /// Handle JavaScript `prompt()` calls
        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn run_text_input_panel(
            &self,
            _webview: &WKWebView,
            prompt: &NSString,
            default_text: Option<&NSString>,
            _frame: &WKFrameInfo,
            completion_handler: &DynBlock<dyn Fn(*mut NSString)>,
        ) {
            let prompt_str = prompt.to_string();
            let default = default_text.map(std::string::ToString::to_string);
            tracing::debug!("Intercepted prompt: {prompt_str}");
            let response =
                self.ivars()
                    .await_dialog(prompt_str, default.clone(), AlertType::Prompt);

            // The prompt text if accepted; null (cancel) if dismissed or timed out.
            let result: *mut NSString = match response {
                Some(r) if r.accepted => {
                    let text = r.prompt_text.or(default).unwrap_or_default();
                    Retained::into_raw(NSString::from_str(&text))
                }
                _ => std::ptr::null_mut(),
            };
            completion_handler.call((result,));
        }

        /// Rekindle patch: forward to the replaced delegate (wry's
        /// `request_media_capture_permission`, wry-0.57.0
        /// src/wkwebview/class/wry_web_view_ui_delegate.rs:127-169). Without one, answer
        /// `Prompt`, which WebKit documents as the result when the method is not implemented
        /// (WKUIDelegate.h, `webView:requestMediaCapturePermissionForOrigin:...`: "If not
        /// implemented, the result is the same as calling the decisionHandler with
        /// WKPermissionDecisionPrompt").
        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn request_media_capture_permission(
            &self,
            webview: &WKWebView,
            origin: &WKSecurityOrigin,
            frame: &WKFrameInfo,
            capture_type: WKMediaCaptureType,
            decision_handler: &DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            let selector = sel!(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:);
            match self.ivars().previous_responding_to(selector) {
                // SAFETY: the previous delegate implements this selector (checked above) and
                // receives exactly the arguments WebKit passed us, on the same (main) thread.
                Some(previous) => unsafe {
                    previous.webView_requestMediaCapturePermissionForOrigin_initiatedByFrame_type_decisionHandler(
                        webview,
                        origin,
                        frame,
                        capture_type,
                        decision_handler,
                    );
                },
                None => decision_handler.call((WKPermissionDecision::Prompt,)),
            }
        }

        /// Rekindle patch: forward to the replaced delegate (wry's `run_file_upload_panel`,
        /// wry-0.57.0 src/wkwebview/class/wry_web_view_ui_delegate.rs:99-124). Without one,
        /// complete with nil, which WebKit documents as the macOS behaviour when the method is
        /// not implemented (WKUIDelegate.h, `webView:runOpenPanelWithParameters:...`: "the web
        /// view will behave as if the user selected the Cancel button").
        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        fn run_open_panel(
            &self,
            webview: &WKWebView,
            parameters: &WKOpenPanelParameters,
            frame: &WKFrameInfo,
            completion_handler: &DynBlock<dyn Fn(*mut NSArray<NSURL>)>,
        ) {
            let selector =
                sel!(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:);
            match self.ivars().previous_responding_to(selector) {
                // SAFETY: the previous delegate implements this selector (checked above) and
                // receives exactly the arguments WebKit passed us, on the same (main) thread.
                Some(previous) => unsafe {
                    previous.webView_runOpenPanelWithParameters_initiatedByFrame_completionHandler(
                        webview,
                        parameters,
                        frame,
                        completion_handler,
                    );
                },
                None => completion_handler.call((std::ptr::null_mut(),)),
            }
        }
    }
);

impl WebDriverUIDelegate {
    pub(super) fn new(
        mtm: MainThreadMarker,
        alert_state: Arc<AlertState>,
        previous: Option<Retained<ProtocolObject<dyn WKUIDelegate>>>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(WebDriverUIDelegateIvars {
            alert_state,
            previous,
        });
        // SAFETY: `init` is NSObject's designated initialiser, called once on a fresh allocation.
        unsafe { msg_send![super(this), init] }
    }
}

pub(super) struct EvalMessageHandlerIvars {
    registry: Arc<EvalResultRegistry>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WdioEvalMessageHandler"]
    #[ivars = EvalMessageHandlerIvars]
    pub(super) struct EvalMessageHandler;

    unsafe impl NSObjectProtocol for EvalMessageHandler {}

    unsafe impl WKScriptMessageHandler for EvalMessageHandler {
        /// Delivers `{ id, result }` posted by the DirectEval wrapper to the waiting executor.
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive_script_message(
            &self,
            _controller: &WKUserContentController,
            message: &WKScriptMessage,
        ) {
            // SAFETY: `body` is valid inside the message callback, on the main thread.
            let body = unsafe { message.body() };
            let json = ns_object_to_json(&body);
            let (Some(id), Some(result)) = (
                json.get("id").and_then(Value::as_str),
                json.get("result").cloned(),
            ) else {
                tracing::warn!("wdioEvalResult message missing id/result");
                return;
            };
            self.ivars().registry.complete(id, result);
        }
    }
);

impl EvalMessageHandler {
    pub(super) fn new(mtm: MainThreadMarker, registry: Arc<EvalResultRegistry>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(EvalMessageHandlerIvars { registry });
        // SAFETY: `init` is NSObject's designated initialiser, called once on a fresh allocation.
        unsafe { msg_send![super(this), init] }
    }
}
