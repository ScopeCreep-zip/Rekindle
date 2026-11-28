//! Every window is built hidden and shown once it is complete (plan WL2).
//!
//! A frameless window built visible opens 37 px taller than its declared
//! inner size on GTK Wayland (login 380×517 instead of 380×480). tao 0.37.1
//! `platform_impl/linux/window.rs` sets the default size (:110), then maps
//! the window (`set_visible`, :191) while it is still decorated, and only
//! then calls `set_decorated(false)` (:192) and installs the Wayland CSD
//! titlebar (:193-197). GTK 3.24 `gtkwindow.c`: that first realize makes
//! GTK's own CSD titlebar (`gtk_window_realize` :7419-7420,
//! `create_decoration` :6191-6213, Wayland CSD per
//! `gtk_window_should_use_csd` :6172-6177) and the first configure request
//! adds its height to the default size (`gtk_window_compute_configure_request_size`
//! :9491-9497 → `gtk_window_update_csd_size` :5196-5228); the late
//! `gtk_window_set_titlebar` on the realized window unrealizes and maps it
//! again (:4240-4247, :4280), and the new request keeps that remembered size
//! (:9509, `gtk_window_get_remembered_size` :6650-6656). X11 uses no CSD, so
//! it never showed.
//!
//! Built hidden, the window is first realized by the show, after the
//! decorations are off and the titlebar is set, as `gtk_window_set_titlebar`'s
//! documentation asks ("set the titlebar before calling gtk_widget_show()"):
//! the window is client-decorated already, so no GTK titlebar is made
//! (:7419), the default size takes no CSD size on an undecorated window
//! (:5207-5209), and the titlebar gets no allocation (:7883-7887). The
//! content area is the declared inner size on every platform.

/// A window builder as [`open`] drives it (a seam for the tests).
pub(super) trait Build: Sized {
    type Window: Show;
    /// The builder with the window's initial visibility set.
    #[must_use]
    fn visible(self, visible: bool) -> Self;
    /// Build the window.
    ///
    /// # Errors
    /// The runtime could not create the window or its webview.
    fn build(self) -> Result<Self::Window, String>;
}

/// The show a new window gets once it is complete (a seam for the tests).
pub(super) trait Show {
    fn show(&self) -> tauri::Result<()>;
}

impl Build for tauri::WebviewWindowBuilder<'_, tauri::Wry, tauri::AppHandle> {
    type Window = tauri::WebviewWindow;
    fn visible(self, visible: bool) -> Self {
        tauri::WebviewWindowBuilder::visible(self, visible)
    }
    fn build(self) -> Result<Self::Window, String> {
        tauri::WebviewWindowBuilder::build(self).map_err(|e| e.to_string())
    }
}

impl Show for tauri::WebviewWindow {
    fn show(&self) -> tauri::Result<()> {
        tauri::WebviewWindow::show(self)
    }
}

/// Build `builder` hidden and show it if `prepare` permits immediate display.
///
/// # Errors
/// The window could not be built. A failed show is logged: the window
/// exists and the next [`super::reveal_window`] shows it.
pub(super) fn open<B: Build>(
    builder: B,
    prepare: impl FnOnce(&B::Window) -> bool,
) -> Result<B::Window, String> {
    let window = builder.visible(false).build()?;
    if prepare(&window) {
        if let Err(e) = window.show() {
            tracing::warn!(error = %e, "open: first show failed");
        }
    }
    Ok(window)
}

/// The Host's local skin read and preparation precede construction, where
/// the native and document reveal deadline starts.
pub(super) async fn after_skin<T>(
    prepare: impl std::future::Future<Output = Result<(), String>>,
    build: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    prepare.await?;
    build()
}

#[cfg(test)]
#[path = "first_show_tests.rs"]
mod tests;
