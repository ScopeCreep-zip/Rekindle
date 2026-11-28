//! Bring an existing window back: shown, restored and focused, on every
//! desktop the app runs on.
//!
//! A window hidden to the tray (`hide()`, the buddy list's close) is an
//! unmapped toplevel; on Wayland it must be mapped again with `show()`
//! (GTK `show_all`, tao-0.37.1 `platform_impl/linux/event_loop.rs:306-311`),
//! and Wayland reports no minimized state back to the client, so the
//! restore cannot be decided from `is_minimized`: `unminimize()` is always
//! sent (`deiconify`, a no-op on a window that is not iconified). tao queues
//! `show()` to the GTK main loop and drops a focus request while GTK still
//! reports the window hidden (`platform_impl/linux/window.rs:582-583`); the
//! show itself maps the window, which is what makes it visible again.

/// The window calls a reveal takes (a seam for the tests).
pub(super) trait Revealable {
    fn show(&self) -> tauri::Result<()>;
    fn unminimize(&self) -> tauri::Result<()>;
    fn set_focus(&self) -> tauri::Result<()>;
}

impl<R: tauri::Runtime> Revealable for tauri::WebviewWindow<R> {
    fn show(&self) -> tauri::Result<()> {
        tauri::WebviewWindow::<R>::show(self)
    }
    fn unminimize(&self) -> tauri::Result<()> {
        tauri::WebviewWindow::<R>::unminimize(self)
    }
    fn set_focus(&self) -> tauri::Result<()> {
        tauri::WebviewWindow::<R>::set_focus(self)
    }
}

/// Show, restore and focus `window` (see the module docs). Failures are
/// logged and never abort the caller.
pub(super) fn reveal(window: &impl Revealable) {
    if let Err(e) = window.show() {
        tracing::warn!(error = %e, "reveal: show failed");
    }
    if let Err(e) = window.unminimize() {
        tracing::trace!(error = %e, "reveal: unminimize failed");
    }
    if let Err(e) = window.set_focus() {
        tracing::warn!(error = %e, "reveal: set_focus failed");
    }
}

/// Show an ordinary window immediately. A skin window uses the same
/// native and document handshake as its first show.
pub fn reveal_window(window: &tauri::WebviewWindow) {
    if super::skin_load::framed(window) {
        super::skin_load::request(window, super::skin_reveal::Show::Raise);
    } else {
        reveal(window);
    }
}

#[cfg(test)]
#[path = "reveal_tests.rs"]
mod tests;
