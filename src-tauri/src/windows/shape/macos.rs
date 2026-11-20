//! macOS shaper: a cursor tracker that toggles click-through as the cursor
//! crosses the silhouette's edge.
//!
//! AppKit has no input-region API: `NSWindow.ignoresMouseEvents` is all or
//! nothing, and a WKWebView window, drawn through a layer tree, is a hit
//! over its whole rectangle whatever its alpha (research §4). The visible
//! shape already comes from alpha (tao 0.37.1 `macos/window.rs:542-557`
//! `setOpaque(false)` + wry's transparent WKWebView). Input follows the
//! silhouette through Tauri's safe API only: `cursor_position` and
//! `set_ignore_cursor_events` (tauri 2.12.1 `webview_window.rs:2025,2260`;
//! tao `macos/util/async.rs:258-262` → `setIgnoresMouseEvents`). No
//! `unsafe`.
//!
//! The tracker is one task per shaped window, owned by the window: it ends
//! when the window is destroyed (its token is cancelled from the window's
//! `Destroyed` event) and then gives the window its input back. While the
//! window is hidden it reads no cursor and keeps input on; Tauri has no
//! hide event, so a hidden window is noticed on the next idle tick.
//!
//! **Edge-tick limitation (recorded in plan SK0):** the state changes at
//! most once per [`CURSOR_TICK`], so a click within one tick of crossing the
//! silhouette's edge can land on the wrong side, on the edge pixels only.
//! Xfire's own 1-bit regions had hard edges too.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tauri::WebviewWindow;
use tokio_util::sync::CancellationToken;

use super::geometry::{local_point, Silhouette};

/// About one display frame.
const CURSOR_TICK: Duration = Duration::from_millis(16);
/// The poll interval while the window is hidden.
const CURSOR_IDLE_TICK: Duration = Duration::from_millis(250);

/// What one tick decides: whether the window should ignore the mouse.
///
/// The window ignores the mouse only while the cursor is inside its
/// rectangle and outside the silhouette; outside the rectangle it takes no
/// events anyway, so input stays on and is ready the moment the cursor
/// comes back over art.
pub(super) fn should_ignore(silhouette: &Silhouette, local: (f64, f64), size: (u32, u32)) -> bool {
    let inside_window = local.0 >= 0.0
        && local.1 >= 0.0
        && local.0 < f64::from(size.0)
        && local.1 < f64::from(size.1);
    inside_window && !silhouette.contains(local.0, local.1)
}

/// The cursor relative to `window` in its device pixels, with its size.
fn sample(window: &WebviewWindow) -> Result<((f64, f64), (u32, u32)), String> {
    let cursor = window.cursor_position().map_err(|e| e.to_string())?;
    let primary = window
        .primary_monitor()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no primary monitor".to_owned())?;
    let position = window.inner_position().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let local = local_point(
        (cursor.x, cursor.y),
        primary.scale_factor(),
        (position.x, position.y),
        scale,
    );
    Ok((local, (size.width, size.height)))
}

/// Set click-through to `want`, recording it in `ignoring` (shared by the
/// tracker and the shaper, so neither acts on a stale state).
pub(super) fn set(window: &WebviewWindow, ignoring: &AtomicBool, want: bool) -> Result<(), String> {
    window
        .set_ignore_cursor_events(want)
        .map_err(|e| e.to_string())?;
    ignoring.store(want, Ordering::Relaxed);
    Ok(())
}

/// Apply `silhouette` under the cursor now, before the tracker's next tick.
pub(super) fn apply_now(
    window: &WebviewWindow,
    silhouette: &Silhouette,
    ignoring: &AtomicBool,
) -> Result<(), String> {
    let (local, size) = sample(window)?;
    set(window, ignoring, should_ignore(silhouette, local, size))
}

/// Run the tracker for `window` until `stop` is cancelled. `current` is
/// the silhouette the shaper last applied; `ignoring` the click-through
/// state last set.
pub(super) fn start(
    window: WebviewWindow,
    current: Arc<Mutex<Option<Silhouette>>>,
    ignoring: Arc<AtomicBool>,
    stop: CancellationToken,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            let visible = window.is_visible().unwrap_or(false);
            let want = if visible {
                let silhouette = current.lock().clone();
                match (silhouette, sample(&window)) {
                    (Some(silhouette), Ok((local, size))) => {
                        should_ignore(&silhouette, local, size)
                    }
                    _ => false,
                }
            } else {
                false
            };
            if want != ignoring.load(Ordering::Relaxed) {
                if let Err(e) = set(&window, &ignoring, want) {
                    tracing::debug!(error = %e, label = window.label(), "cursor toggle failed");
                }
            }
            let tick = if visible {
                CURSOR_TICK
            } else {
                CURSOR_IDLE_TICK
            };
            tokio::select! {
                () = stop.cancelled() => break,
                () = tokio::time::sleep(tick) => {}
            }
        }
        if ignoring.load(Ordering::Relaxed) {
            let _ = set(&window, &ignoring, false);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::super::geometry::ShapeRect;
    use super::*;

    #[test]
    fn ignores_only_over_holes_inside_the_window() {
        // A rounded-corner stand-in: the top-left 8×8 is cut away.
        let s = Silhouette::new(&[
            ShapeRect {
                x: 8,
                y: 0,
                w: 92,
                h: 8,
            },
            ShapeRect {
                x: 0,
                y: 8,
                w: 100,
                h: 92,
            },
        ])
        .expect("valid");
        assert!(should_ignore(&s, (2.0, 2.0), (100, 100)), "corner hole");
        assert!(!should_ignore(&s, (50.0, 2.0), (100, 100)), "top band art");
        assert!(!should_ignore(&s, (2.0, 50.0), (100, 100)), "body");
        assert!(
            !should_ignore(&s, (-5.0, 2.0), (100, 100)),
            "outside the window"
        );
        assert!(
            !should_ignore(&s, (100.0, 50.0), (100, 100)),
            "past the right edge"
        );
    }
}
