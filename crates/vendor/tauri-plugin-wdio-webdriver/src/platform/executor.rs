use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{Listener, LogicalSize, PhysicalPosition, PhysicalSize, Runtime, WebviewWindow};

use crate::server::response::WebDriverErrorResponse;

/// Element bounding rectangle
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ElementRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Window rectangle (position and size)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WindowRect {
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub(crate) fn physical_window_rect_to_logical(
    position: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    scale_factor: f64,
) -> WindowRect {
    let position = position.to_logical::<i32>(scale_factor);
    let size = size.to_logical::<u32>(scale_factor);

    WindowRect {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    }
}

pub(crate) fn physical_window_chrome_to_logical(
    outer: PhysicalSize<u32>,
    inner: PhysicalSize<u32>,
    scale_factor: f64,
) -> LogicalSize<u32> {
    PhysicalSize::new(
        outer.width.saturating_sub(inner.width),
        outer.height.saturating_sub(inner.height),
    )
    .to_logical(scale_factor)
}

pub(crate) const WINDOW_CHANGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// Physical-to-logical rounding can leave a requested logical value off by one pixel.
const LOGICAL_ROUNDING_TOLERANCE: u32 = 1;

/// Apply a native window change and wait, up to `timeout`, until `is_applied` confirms it.
///
/// `event_name` only triggers a re-check, as the event can predate this change or come from
/// another window. `is_applied` should return `true` if it can't read the window state, rather
/// than wait out the timeout.
///
/// Only the native call failing is an error; a change the window manager clamps or ignores
/// just times out.
pub(crate) async fn apply_window_change<R, F, A>(
    window: &WebviewWindow<R>,
    event_name: &'static str,
    operation: &'static str,
    timeout: std::time::Duration,
    change: F,
    is_applied: A,
) -> Result<(), WebDriverErrorResponse>
where
    R: Runtime,
    F: FnOnce() -> tauri::Result<()>,
    A: Fn() -> bool,
{
    let notify = std::sync::Arc::new(tokio::sync::Notify::new());
    let listener_id = window.listen(event_name, {
        let notify = std::sync::Arc::clone(&notify);
        move |_| notify.notify_one()
    });

    if let Err(error) = change() {
        window.unlisten(listener_id);
        return Err(WebDriverErrorResponse::unknown_error(&format!(
            "Failed to {operation}: {error}"
        )));
    }

    if !wait_until_applied(&notify, timeout, is_applied).await {
        tracing::debug!("Timed out waiting to {operation}");
    }

    window.unlisten(listener_id);
    Ok(())
}

/// Wait until `is_applied` holds, re-checking on each `notify`; returns `false` on timeout.
async fn wait_until_applied(
    notify: &tokio::sync::Notify,
    timeout: std::time::Duration,
    is_applied: impl Fn() -> bool,
) -> bool {
    tokio::time::timeout(timeout, async {
        while !is_applied() {
            notify.notified().await;
        }
    })
    .await
    .is_ok()
}

pub(crate) fn has_logical_position<R: Runtime>(window: &WebviewWindow<R>, x: i32, y: i32) -> bool {
    match (window.outer_position(), window.scale_factor()) {
        (Ok(position), Ok(scale_factor)) => {
            let position = position.to_logical::<i32>(scale_factor);
            position.x.abs_diff(x) <= LOGICAL_ROUNDING_TOLERANCE
                && position.y.abs_diff(y) <= LOGICAL_ROUNDING_TOLERANCE
        }
        _ => true,
    }
}

pub(crate) fn has_logical_inner_size<R: Runtime>(
    window: &WebviewWindow<R>,
    size: LogicalSize<u32>,
) -> bool {
    match (window.inner_size(), window.scale_factor()) {
        (Ok(inner_size), Ok(scale_factor)) => {
            let inner_size = inner_size.to_logical::<u32>(scale_factor);
            inner_size.width.abs_diff(size.width) <= LOGICAL_ROUNDING_TOLERANCE
                && inner_size.height.abs_diff(size.height) <= LOGICAL_ROUNDING_TOLERANCE
        }
        _ => true,
    }
}

#[cfg(test)]
#[path = "executor_tests.rs"]
mod tests;

/// Frame identifier for switching frames
#[derive(Debug, Clone)]
pub enum FrameId {
    /// Frame by index
    Index(u32),
    /// Frame by element reference (`js_var`)
    Element(String),
}

/// Pointer event type
#[derive(Debug, Clone, Copy)]
pub enum PointerEventType {
    Down,
    Up,
    Move,
}

/// Cookie data
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default)]
    pub secure: bool,
    #[serde(default, rename = "httpOnly")]
    pub http_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "sameSite")]
    pub same_site: Option<String>,
}

/// Print options for PDF generation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrintOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "pageWidth")]
    pub page_width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "pageHeight")]
    pub page_height: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "marginTop")]
    pub margin_top: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "marginBottom")]
    pub margin_bottom: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "marginLeft")]
    pub margin_left: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "marginRight")]
    pub margin_right: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "shrinkToFit")]
    pub shrink_to_fit: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "pageRanges")]
    pub page_ranges: Option<Vec<String>>,
}

/// The modifier flags a key event carries; `key_input::HeldModifiers` derives them from the held keys
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModifierState {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Platform-agnostic trait for `WebView` operations.
/// Each platform (macOS, Windows, Linux) implements this trait.
#[async_trait]
pub trait PlatformExecutor<R: Runtime>: Send + Sync {
    // =========================================================================
    // Window Access
    // =========================================================================

    /// Get a reference to the underlying window
    fn window(&self) -> &WebviewWindow<R>;

    /// Get the script timeout in milliseconds
    fn script_timeout_ms(&self) -> u64;

    // =========================================================================
    // Core JavaScript Execution
    // =========================================================================

    /// Execute JavaScript and return the result as JSON
    async fn evaluate_js(&self, script: &str) -> Result<Value, WebDriverErrorResponse>;

    /// Execute asynchronous JavaScript with callback.
    ///
    /// Each platform must implement this using native message handlers.
    async fn execute_async_script(
        &self,
        script: &str,
        args: &[Value],
    ) -> Result<Value, WebDriverErrorResponse>;

    /// Execute a pre-wrapped async DirectEval script (the `/wdio/eval` channel).
    ///
    /// The script uses the W3C async-callback contract (`arguments[last]` is the done callback).
    /// Defaults to `execute_async_script`; a platform may override to avoid a long-lived native
    /// completion handler (see the macOS executor).
    async fn execute_direct_eval(&self, script: &str) -> Result<Value, WebDriverErrorResponse> {
        self.execute_async_script(script, &[]).await
    }

    // =========================================================================
    // Screenshots
    // =========================================================================

    /// Take screenshot of the page, returns base64-encoded PNG
    async fn take_screenshot(&self) -> Result<String, WebDriverErrorResponse>;

    /// Take screenshot of a specific element, returns base64-encoded PNG
    async fn take_element_screenshot(&self, js_var: &str)
        -> Result<String, WebDriverErrorResponse>;

    // =========================================================================
    // Print
    // =========================================================================

    /// Print page to PDF, returns base64-encoded PDF
    async fn print_page(&self, options: PrintOptions) -> Result<String, WebDriverErrorResponse>;

    // =========================================================================
    // Native key input (Rekindle, plan E7.14 S5)
    // =========================================================================

    /// Press and release `key` (a GDK key name such as `Tab`), with Shift
    /// held when `shift`, as native input the engine treats as the user's:
    /// the page sees a trusted event and the engine performs the key's
    /// default action (Tab moves focus). The W3C key actions dispatch
    /// synthetic `KeyboardEvent`s, which never get a default action.
    async fn send_native_key(&self, key: &str, shift: bool) -> Result<(), WebDriverErrorResponse>;
}
