//! Give the selected window keyboard focus before a trusted key reaches it.
//!
//! WebDriver's Switch To Window makes a window the current top-level browsing
//! context, but it moves no native focus: with several toplevels open, GTK
//! keeps focus on whichever the window manager last activated, and a trusted
//! key (linux_keys.rs) goes only to the focused widget. A user's click both
//! raises the window and focuses the page, so this does the same: present the
//! toplevel natively, focus its webview, and wait until GTK reports keyboard
//! focus (`gtk_widget_has_focus` is true only while the widget is its
//! toplevel's focus widget and that toplevel holds the keyboard). If it never
//! does, the key is refused with a named error: there is no synthetic-key
//! fallback.

use std::time::Duration;

use gtk::prelude::*;
use tauri::{Runtime, WebviewWindow};
use tokio::sync::oneshot;

use crate::server::response::WebDriverErrorResponse;

/// How long the window manager has to hand the presented window focus.
const FOCUS_WAIT: Duration = Duration::from_secs(2);

/// How often focus is read again while waiting.
const FOCUS_POLL: Duration = Duration::from_millis(25);

/// Present `window` and focus its webview until GTK says it holds keyboard
/// focus, or refuse once `FOCUS_WAIT` has passed.
pub(super) async fn hold_keyboard_focus<R: Runtime>(
    window: &WebviewWindow<R>,
) -> Result<(), WebDriverErrorResponse> {
    let deadline = tokio::time::Instant::now() + FOCUS_WAIT;
    loop {
        if focus_once(window).await? {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(WebDriverErrorResponse::element_not_interactable(
                "The selected GTK window did not take keyboard focus when presented",
            ));
        }
        tokio::time::sleep(FOCUS_POLL).await;
    }
}

/// Whether the webview holds keyboard focus now; if not, present its toplevel
/// and focus it, so a later read can find it focused. Runs on GTK's thread.
async fn focus_once<R: Runtime>(window: &WebviewWindow<R>) -> Result<bool, WebDriverErrorResponse> {
    let (tx, rx) = oneshot::channel();
    window
        .with_webview(move |webview| {
            let view = webview.inner();
            let focused = view.has_focus();
            if !focused {
                if let Some(toplevel) = view
                    .toplevel()
                    .and_then(|w| w.downcast::<gtk::Window>().ok())
                {
                    toplevel.present();
                }
                view.grab_focus();
            }
            let _ = tx.send(focused);
        })
        .map_err(|error| WebDriverErrorResponse::unknown_error(&error.to_string()))?;
    rx.await
        .map_err(|_| WebDriverErrorResponse::unknown_error("Native focus channel closed"))
}
