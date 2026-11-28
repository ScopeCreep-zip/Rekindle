//! Linux shaper (Wayland and X11): the silhouette becomes the GtkWindow's
//! input shape. Safe gtk-rs only. Research with citations:
//! `.claude/plans/standards-remediation/research-linux-transparency-2026-11-26.md`.
//!
//! **The shape is set through the widget, never on the `GdkWindow`.**
//! tao turns client-side decorations on for an undecorated Wayland window
//! (tao 0.37.1 `linux/window.rs:192-196`, `set_titlebar`), and GTK 3.24
//! rewrites a client-decorated window's GdkWindow input shape on every size
//! allocation from its own decoration rectangle (`gtkwindow.c:7270-7284`,
//! `gtk_widget_set_csd_input_shape`). That rewrite keeps only the region
//! given to `gtk_widget_input_shape_combine_region`, intersected with the
//! decoration rectangle (`gtkwidget.c:13063-13100`,
//! `gtk_widget_update_input_shape`). A region set directly with
//! `gdk_window_input_shape_combine_region` was overwritten by the next
//! allocation: measured on COSMIC with `WAYLAND_DEBUG=client`, every
//! `wl_surface.set_input_region` was the whole window plus GTK's resize
//! border. Through the widget:
//! - GTK keeps the region on the widget (`quark_input_shape_info`,
//!   `gtkwidget.c:13127-13141`) and re-sends it after every allocation, on
//!   Wayland as `wl_surface.set_input_region` on commit
//!   (`gdkwindow-wayland.c:1541-1562, 3956-3975`) and on X11 as the SHAPE
//!   extension's input shape. One path serves both backends; outside the
//!   region clicks reach the window below.
//! - A window not realized yet takes the region too: GTK applies the kept
//!   input shape in `gtk_widget_realize`
//!   (`gtkwidget.c:5534-5540`), so the first shape is never lost and no
//!   `realize` or `size-allocate` handler is needed.
//! - The region is in widget coordinates, which are the GdkWindow's:
//!   `decorations(false)` and `shadow(false)` leave no CSD margin inside
//!   the window (the `-10` rectangle measured above is GTK's resize border
//!   outside the window, which the intersection drops).
//!
//! **No bounding shape is set, on any backend.** The page paints alpha 0
//! outside the frame art and tao/wry give the window an RGBA visual, a
//! clear draw and a transparent webview background (tao 0.37.1
//! `linux/window.rs:141-153`, `event_loop.rs:927-951`; wry 0.57.0
//! `webkitgtk/mod.rs:292-294`), so the silhouette is already drawn; only
//! input needs shaping. Measured on the Linux machine (Pop!_OS, cosmic-comp
//! 1.0.0, git `8cb1802`, 2026-11-26): a control RGBA GTK3 window cleared to
//! alpha 0 composites transparently under both XWayland and native Wayland,
//! our window's Smithay frame under `GDK_BACKEND=x11` is already depth 32
//! (client 32, root 24), and giving the control window an X11 bounding
//! shape as this module used to (`gdk_window.shape_combine_region`) turned
//! the cut-away area solid black (0,0,0) while its alpha-0 pixels inside
//! the shape still showed the desktop: Xwayland never forwards the bounding
//! shape (`xwayland-screen.c:342-366`) and Smithay's frame shows unpainted
//! there. GTK's Wayland backend ignores the bounding shape altogether
//! (`gdkwindow-wayland.c:3949-3954`). An X server with no compositing
//! manager does not blend the RGBA visual, so clear areas there draw as
//! the page paints them; that session has no transparency for any client.
//!
//! **XWayland limits (compositor side, not fixable here):** Xwayland
//! forwards an X input shape to `wl_surface.set_input_region` only for the
//! window owning the `wl_surface` (`xwayland-window.c:1515, 2105-2136`), and
//! under a reparenting window manager that is the frame, onto which Smithay
//! copies no client shape; so click-through in clear areas cannot work
//! under XWayland on COSMIC. Per-pixel alpha under XWayland depends on the
//! frame window's depth: Smithay `5de53056b5` (cosmic-comp epoch-1.0.14 and
//! later) gives the frame the client's 32-bit visual; a 24-bit frame draws
//! clear areas black.
//!
//! tao's `set_ignore_cursor_events` is never called here: on Linux it
//! replaces the widget input shape with `None` when switched off (tao
//! 0.37.1 `linux/event_loop.rs:452-461`), which would undo the silhouette.

use gtk::cairo::{RectangleInt, Region};
use gtk::prelude::WidgetExt;
use tauri::WebviewWindow;

use super::geometry::{self, Silhouette};

/// The cairo region handed to the widget: the silhouette in GDK's logical
/// coordinates, one rectangle per silhouette rectangle.
pub(super) fn region(silhouette: &Silhouette, scale: i32) -> Region {
    let rects: Vec<RectangleInt> = geometry::to_logical(silhouette, scale)
        .into_iter()
        .map(|(x, y, w, h)| RectangleInt::new(x, y, w, h))
        .collect();
    Region::create_rectangles(&rects)
}

/// Run `shape` on `window`'s GtkWindow on the GTK main thread (gtk-rs
/// types are main-thread only; tauri `webview_window.rs` `gtk_window` doc).
/// The GtkWindow exists from creation, realized or not.
fn on_gtk_window<F>(window: &WebviewWindow, shape: F) -> Result<(), String>
where
    F: FnOnce(&gtk::ApplicationWindow) + Send + 'static,
{
    let target = window.clone();
    window
        .run_on_main_thread(move || match target.gtk_window() {
            Ok(gtk_window) => shape(&gtk_window),
            Err(e) => {
                tracing::warn!(error = %e, label = target.label(), "window shape not set");
            }
        })
        .map_err(|e| e.to_string())
}

/// Cut `window` to `silhouette`.
pub(super) fn apply(window: &WebviewWindow, silhouette: Silhouette) -> Result<(), String> {
    on_gtk_window(window, move |gtk_window| {
        let region = region(&silhouette, gtk_window.scale_factor());
        gtk_window.input_shape_combine_region(Some(&region));
    })
}

/// Give `window` back its whole rectangle: no app input region, so GTK
/// uses its decoration rectangle, or the whole window.
pub(super) fn clear(window: &WebviewWindow) -> Result<(), String> {
    on_gtk_window(window, |gtk_window| {
        gtk_window.input_shape_combine_region(None);
    })
}

#[cfg(test)]
#[path = "linux_tests.rs"]
mod tests;
