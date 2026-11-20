//! Linux shaper (X11 and Wayland): the silhouette becomes the GDK window's
//! input shape and, on X11, its bounding shape. Safe gtk-rs only.
//!
//! - `gdk::Window::input_shape_combine_region` (gdk 0.18.2
//!   `auto/window.rs:661`): on Wayland GTK 3.24 stores the region and
//!   `gdk_wayland_window_sync_input_region` sends it as
//!   `wl_surface.set_input_region` on commit (`gdkwindow-wayland.c:1541-1562,
//!   3956-3975`); on X11 it is the SHAPE extension's input shape. Outside it
//!   clicks reach the window below.
//! - `gdk::Window::shape_combine_region` (`auto/window.rs:1178`): the X11
//!   SHAPE bounding region, so a session without a compositor (no RGBA
//!   visual) still shows the 1-bit silhouette. GTK 3.24's Wayland backend
//!   leaves it empty (`gdkwindow-wayland.c:3949-3954`), so it is applied on
//!   X11 only, where it means something.
//!
//! tao's `set_ignore_cursor_events` is never called here: on Linux it
//! replaces the input shape with `None` when switched off (tao 0.37.1
//! `linux/event_loop.rs:452-461`), which would undo the silhouette.

use gtk::cairo::{RectangleInt, Region};
use gtk::glib::prelude::ObjectExt;
use gtk::prelude::WidgetExt;
use tauri::WebviewWindow;

use super::geometry::{self, Silhouette};

/// GDK's X11 display class (`gdk/x11/gdkdisplay-x11.c`,
/// `G_DEFINE_TYPE (GdkX11Display, ...)`).
const X11_DISPLAY_TYPE: &str = "GdkX11Display";

/// A cairo region from the silhouette in GDK's logical coordinates.
pub(super) fn region(silhouette: &Silhouette, scale: i32) -> Region {
    let rects: Vec<RectangleInt> = geometry::to_logical(silhouette, scale)
        .into_iter()
        .map(|(x, y, w, h)| RectangleInt::new(x, y, w, h))
        .collect();
    Region::create_rectangles(&rects)
}

fn gdk_window(window: &WebviewWindow) -> Result<gtk::gdk::Window, String> {
    window
        .gtk_window()
        .map_err(|e| e.to_string())?
        .window()
        .ok_or_else(|| "the window is not realized yet".to_owned())
}

/// Cut `window` to `silhouette`. Runs on the GTK main thread (gtk-rs types
/// are main-thread only; tauri `webview_window.rs` `gtk_window` doc).
pub(super) fn apply(window: &WebviewWindow, silhouette: Silhouette) -> Result<(), String> {
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            let gdk = match gdk_window(&target) {
                Ok(gdk) => gdk,
                Err(e) => {
                    tracing::warn!(error = %e, label = target.label(), "window shape not applied");
                    return;
                }
            };
            let region = region(&silhouette, gdk.scale_factor());
            gdk.input_shape_combine_region(&region, 0, 0);
            if gdk.display().type_().name() == X11_DISPLAY_TYPE {
                gdk.shape_combine_region(Some(&region), 0, 0);
            }
        })
        .map_err(|e| e.to_string())
}

/// Give `window` back its whole rectangle: the full input region (one
/// rectangle covering the window, as GDK has no "infinite" value to pass
/// through the safe API's `&Region`) and no X11 bounding shape.
pub(super) fn clear(window: &WebviewWindow) -> Result<(), String> {
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            let Ok(gdk) = gdk_window(&target) else {
                return;
            };
            let whole = Region::create_rectangle(&RectangleInt::new(
                0,
                0,
                gdk.width().max(1),
                gdk.height().max(1),
            ));
            gdk.input_shape_combine_region(&whole, 0, 0);
            if gdk.display().type_().name() == X11_DISPLAY_TYPE {
                gdk.shape_combine_region(None, 0, 0);
            }
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::geometry::Rect;
    use super::*;

    #[test]
    fn region_keeps_the_silhouette_and_drops_the_cut_away_corner() {
        let s = Silhouette::new(&[
            Rect {
                x: 20,
                y: 0,
                w: 10,
                h: 10,
            },
            Rect {
                x: 0,
                y: 10,
                w: 50,
                h: 40,
            },
        ])
        .expect("valid");
        let r = region(&s, 1);
        assert!(r.contains_point(25, 5));
        assert!(r.contains_point(0, 10));
        assert!(!r.contains_point(5, 5));
        assert!(!r.contains_point(50, 20));
        let r2 = region(&s, 2);
        assert!(r2.contains_point(10, 0));
        assert!(!r2.contains_point(25, 20));
    }
}
