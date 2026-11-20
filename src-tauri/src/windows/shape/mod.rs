//! Shaped windows (plan SK0 S3): every window is cut to its skin's
//! silhouette, so the pixels outside the frame art are neither drawn nor
//! hit, as Xfire's 1-bit window regions were (research
//! `scratchpad/shaped/research.md` §1, §7).
//!
//! The silhouette is a list of device-pixel rectangles ([`ShapeRect`]), computed
//! in Rust from the window's size and scale by `rekindle_skin::shape::
//! silhouette` over the layout of the window's frame in the shown skin's
//! validated package (the Host's `SkinFrames`, [`skin`]); a window with no
//! frame, or a skin with no layout for it, keeps its whole rectangle,
//! without the shadow. One function cuts a window, whatever the platform:
//! - Linux: GDK input shape (Wayland input region, X11 SHAPE input) plus
//!   the X11 bounding shape ([`linux`]);
//! - Windows: `SetWindowRgn` ([`win32_region`]);
//! - macOS: a cursor tracker toggling click-through ([`macos`]).
//!
//! **The webview never shapes a window** (plan SK0 S4, threat model W4):
//! no IPC command takes a region or a cursor-ignore flag, so an XSS cannot
//! make a window invisible or click-through. `cargo xtask
//! check-no-shape-ipc` holds every `#[tauri::command]` to that.
//!
//! Recomputed on: creation ([`attach`]), `Resized` (which maximize,
//! unmaximize and restore all raise; Tauri has no separate maximize event),
//! `ScaleFactorChanged`, and a skin change ([`refresh`] from
//! `rebind::follow_event` on `SystemEvent::SkinChanged`, which refetches
//! the frames and reshapes every window).

mod geometry;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod skin;
#[cfg(windows)]
mod win32_region;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_types::dto::skin::SkinWindow;
use tauri::{AppHandle, Manager, WebviewWindow, WindowEvent};

pub use geometry::{ShapeRect, Silhouette};
pub use skin::{frame_of, refresh};

/// One shaped window: the skin frame it wears and the silhouette applied.
struct Shaped {
    frame: Mutex<Option<SkinWindow>>,
    current: Arc<Mutex<Option<Silhouette>>>,
    #[cfg(target_os = "macos")]
    ignoring: Arc<std::sync::atomic::AtomicBool>,
    #[cfg(target_os = "macos")]
    stop: tokio_util::sync::CancellationToken,
}

/// Every shaped window, by label. Managed on the app on first use.
#[derive(Default)]
struct ShapeRegistry {
    windows: Mutex<HashMap<String, Arc<Shaped>>>,
    /// The shown skin's frames (empty until the Host answers).
    frames: Mutex<Arc<skin::SkinFrameMap>>,
    /// The first fetch of the frames has been asked for.
    requested: AtomicBool,
}

fn registry(app: &AppHandle) -> tauri::State<'_, ShapeRegistry> {
    if app.try_state::<ShapeRegistry>().is_none() {
        app.manage(ShapeRegistry::default());
    }
    app.state::<ShapeRegistry>()
}

/// The registry entry for `window`, made (with its macOS tracker) if new.
fn ensure(window: &WebviewWindow, frame: Option<SkinWindow>) -> Arc<Shaped> {
    let app = window.app_handle();
    let registry = registry(app);
    let mut windows = registry.windows.lock();
    if let Some(shaped) = windows.get(window.label()) {
        return Arc::clone(shaped);
    }
    let shaped = Arc::new(Shaped {
        frame: Mutex::new(frame),
        current: Arc::new(Mutex::new(None)),
        #[cfg(target_os = "macos")]
        ignoring: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        #[cfg(target_os = "macos")]
        stop: tokio_util::sync::CancellationToken::new(),
    });
    #[cfg(target_os = "macos")]
    macos::start(
        window.clone(),
        Arc::clone(&shaped.current),
        Arc::clone(&shaped.ignoring),
        shaped.stop.clone(),
    );
    windows.insert(window.label().to_owned(), Arc::clone(&shaped));
    shaped
}

/// Cut `window` to the silhouette `rects` (device pixels, relative to the
/// window's top-left corner).
///
/// # Errors
/// The rectangles are empty or over the cap ([`Silhouette::new`]), or the
/// platform refused the shape.
pub fn apply_shape(window: &WebviewWindow, rects: &[ShapeRect]) -> Result<(), String> {
    let silhouette = Silhouette::new(rects)?;
    let shaped = ensure(window, None);
    *shaped.current.lock() = Some(silhouette.clone());
    platform_apply(window, &silhouette)
}

/// Give `window` back its whole rectangle, drawn and hit everywhere.
///
/// # Errors
/// The platform refused to clear the shape.
pub fn clear_shape(window: &WebviewWindow) -> Result<(), String> {
    let shaped = ensure(window, None);
    *shaped.current.lock() = None;
    platform_clear(window)
}

#[cfg(target_os = "linux")]
fn platform_apply(window: &WebviewWindow, silhouette: &Silhouette) -> Result<(), String> {
    linux::apply(window, silhouette.clone())
}

#[cfg(target_os = "linux")]
fn platform_clear(window: &WebviewWindow) -> Result<(), String> {
    linux::clear(window)
}

#[cfg(windows)]
fn platform_apply(window: &WebviewWindow, silhouette: &Silhouette) -> Result<(), String> {
    let target = window.clone();
    let silhouette = silhouette.clone();
    window
        .run_on_main_thread(move || {
            let result = target
                .hwnd()
                .map_err(|e| e.to_string())
                .and_then(|hwnd| win32_region::set(hwnd, &silhouette));
            if let Err(e) = result {
                tracing::warn!(error = %e, label = target.label(), "window region not set");
            }
        })
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
fn platform_clear(window: &WebviewWindow) -> Result<(), String> {
    let target = window.clone();
    window
        .run_on_main_thread(move || {
            let result = target
                .hwnd()
                .map_err(|e| e.to_string())
                .and_then(win32_region::clear);
            if let Err(e) = result {
                tracing::warn!(error = %e, label = target.label(), "window region not cleared");
            }
        })
        .map_err(|e| e.to_string())
}

/// macOS: the tracker follows `current`, which the caller has set; this
/// applies the new silhouette at once under the cursor instead of a tick
/// later.
#[cfg(target_os = "macos")]
fn platform_apply(window: &WebviewWindow, silhouette: &Silhouette) -> Result<(), String> {
    let shaped = ensure(window, None);
    macos::apply_now(window, silhouette, &shaped.ignoring)
}

#[cfg(target_os = "macos")]
fn platform_clear(window: &WebviewWindow) -> Result<(), String> {
    let shaped = ensure(window, None);
    macos::set(window, &shaped.ignoring, false)
}

/// Recompute and apply `window`'s silhouette for its current size and
/// scale. A size with no silhouette (a minimized window's zero area, a
/// provider error) leaves the window whole rather than keeping a stale
/// shape cut for another size, which could cut into its content.
pub fn reshape(window: &WebviewWindow) {
    let shaped = ensure(window, None);
    let frame = *shaped.frame.lock();
    let frames = Arc::clone(&registry(window.app_handle()).frames.lock());
    let computed = window
        .inner_size()
        .map_err(|e| e.to_string())
        .and_then(|size| {
            let scale = window.scale_factor().map_err(|e| e.to_string())?;
            skin::compute(&frames, frame, size, scale)
        });
    let result = match computed {
        Ok(silhouette) => {
            if shaped.current.lock().as_ref() == Some(&silhouette) {
                return;
            }
            apply_shape(window, silhouette.rects())
        }
        Err(e) => {
            tracing::debug!(error = %e, label = window.label(), "no silhouette; window left whole");
            clear_shape(window)
        }
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, label = window.label(), "window shape not updated");
    }
}

/// Shape a new window wearing `frame` ([`frame_of`] its kind) and keep it
/// shaped: resize (and so maximize/unmaximize) and scale changes recompute
/// it; destruction drops it and stops its tracker. The first window asks
/// the Host for the shown skin's frames ([`refresh`]).
pub fn attach(window: &WebviewWindow, frame: Option<SkinWindow>) {
    let shaped = ensure(window, frame);
    *shaped.frame.lock() = frame;
    if !registry(window.app_handle())
        .requested
        .swap(true, Ordering::AcqRel)
    {
        refresh(window.app_handle());
    }
    let target = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => reshape(&target),
        WindowEvent::Destroyed => forget(&target),
        _ => {}
    });
    reshape(window);
}

/// Drop `window`'s entry and stop its tracker.
fn forget(window: &WebviewWindow) {
    let removed = registry(window.app_handle())
        .windows
        .lock()
        .remove(window.label());
    #[cfg(target_os = "macos")]
    if let Some(shaped) = removed {
        shaped.stop.cancel();
    }
    #[cfg(not(target_os = "macos"))]
    drop(removed);
}

/// Keep the shown skin's `frames` and recompute every shaped window from
/// them.
fn set_frames(app: &AppHandle, frames: skin::SkinFrameMap) {
    *registry(app).frames.lock() = Arc::new(frames);
    reshape_all(app);
}

/// Recompute every shaped window from the frames held.
fn reshape_all(app: &AppHandle) {
    let labels: Vec<String> = registry(app).windows.lock().keys().cloned().collect();
    for label in labels {
        if let Some(window) = app.get_webview_window(&label) {
            reshape(&window);
        }
    }
}
