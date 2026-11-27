//! AppKit and Win32 sample the client allocation once per native UI turn.
//! Pack selection is the only operation that requests a preferred size.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{LogicalSize, Manager, WebviewWindow};

use super::{
    desired, fitting, is_current, is_latest, publish, report_fit, sizing, skin, Shaped, Silhouette,
};

const SIZE_WAIT: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(super) struct State {
    selected: u64,
    request: Option<(LogicalSize<f64>, Instant)>,
    deferred: Option<LogicalSize<f64>>,
    applied: Option<Stamp>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Stamp {
    serial: u64,
    width: u32,
    height: u32,
    scale: u64,
    fixed: bool,
    #[cfg(windows)]
    surface: usize,
}

pub(super) fn queue(window: &WebviewWindow, shaped: &Arc<Shaped>) {
    if shaped.queued.swap(true, Ordering::AcqRel) {
        return;
    }
    let window = window.clone();
    let scoped = Arc::clone(shaped);
    shaped.spawn("sample native window allocation", async move {
        tokio::task::yield_now().await;
        let target = window.clone();
        let turn = Arc::clone(&scoped);
        if let Err(error) = window.run_on_main_thread(move || {
            turn.queued.store(false, Ordering::Release);
            if !is_current(&target, &turn) {
                return;
            }
            if let Err(error) = allocate(&target, &turn) {
                tracing::warn!(%error, label = target.label(), "native window allocation failed");
            }
        }) {
            scoped.queued.store(false, Ordering::Release);
            tracing::warn!(%error, label = window.label(), "window allocation not queued");
        }
    });
}

fn request_after(window: &WebviewWindow, shaped: &Arc<Shaped>) {
    let target = window.clone();
    let scoped = Arc::clone(shaped);
    let stop = shaped.tasks.token();
    shaped.spawn("wait for native window size", async move {
        tokio::select! {
            () = stop.cancelled() => {}
            () = tokio::time::sleep(SIZE_WAIT) => queue(&target, &scoped),
        }
    });
}

fn select_size(
    window: &WebviewWindow,
    shaped: &Arc<Shaped>,
    generation: &super::Generation,
    fixed: bool,
    state: &mut State,
) -> tauri::Result<()> {
    let mut request = None;
    if is_latest(window.app_handle(), generation) && state.selected != generation.serial {
        state.selected = generation.serial;
        let sizing = sizing(generation, shaped.frame);
        window.set_size_constraints(sizing.constraints())?;
        request = Some(LogicalSize::new(sizing.wanted.0, sizing.wanted.1));
        state.request = None;
    } else if !fixed {
        request = state.deferred.take();
    }
    if let Some(size) = request {
        if fixed {
            state.deferred = Some(size);
        } else {
            state.deferred = None;
            window.set_size(size)?;
            state.request = Some((size, Instant::now() + SIZE_WAIT));
            request_after(window, shaped);
        }
    }
    Ok(())
}

fn allocate(window: &WebviewWindow, shaped: &Arc<Shaped>) -> Result<(), String> {
    let mut native = shaped.native.lock();
    if !window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(false) {
        native.applied = None;
        #[cfg(target_os = "macos")]
        {
            *shaped.current.lock() = None;
            super::macos::set(window, &shaped.ignoring, false)?;
        }
        return Ok(());
    }
    let generation = desired(window.app_handle());
    let fixed = window.is_maximized().unwrap_or(false) || window.is_fullscreen().unwrap_or(false);
    select_size(window, shaped, &generation, fixed, &mut native).map_err(|e| e.to_string())?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    if size.width == 0 || size.height == 0 || !scale.is_finite() || scale <= 0.0 {
        return Ok(());
    }
    // Native clients report rounded device pixels. Compare the round-trip
    // logical canvas to the integer pack minimum, as the shaper does.
    let actual = size.to_logical::<u32>(scale);
    let override_size = native
        .request
        .is_some_and(|(want, _)| want.to_physical::<u32>(scale) != size);
    if native
        .request
        .is_some_and(|(_, end)| override_size && !fixed && Instant::now() < end)
    {
        return Ok(());
    }
    native.request = None;
    let canvas = (f64::from(actual.width), f64::from(actual.height));
    let fitted = fitting(window, shaped, &generation, canvas);
    let stamp = Stamp {
        serial: fitted.as_ref().map_or(0, |fit| fit.serial),
        width: size.width,
        height: size.height,
        scale: scale.to_bits(),
        fixed,
        #[cfg(windows)]
        surface: window.hwnd().map_err(|error| error.to_string())?.0.addr(),
    };
    if native.applied != Some(stamp) {
        let silhouette = fitted
            .as_ref()
            .map(|fit| skin::compute(&fit.frames, Some(shaped.frame), size, scale))
            .transpose()?;
        apply(window, shaped, silhouette)?;
        native.applied = Some(stamp);
        tracing::debug!(
            label = window.label(),
            generation = stamp.serial,
            ?size,
            scale,
            fixed,
            override_size,
            "applied native window allocation"
        );
    }
    let fits = fitted.is_some();
    publish(window, shaped, &generation, fitted);
    report_fit(window, shaped, &generation, fits, canvas);
    Ok(())
}

#[cfg(windows)]
fn apply(window: &WebviewWindow, _: &Shaped, silhouette: Option<Silhouette>) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?;
    match silhouette {
        Some(shape) => super::win32_region::set(hwnd, &shape),
        None => super::win32_region::clear(hwnd),
    }
}

#[cfg(target_os = "macos")]
fn apply(
    window: &WebviewWindow,
    scoped: &Shaped,
    silhouette: Option<Silhouette>,
) -> Result<(), String> {
    if let Some(shape) = silhouette.as_ref() {
        super::macos::apply_now(window, shape, &scoped.ignoring)?;
    } else {
        super::macos::set(window, &scoped.ignoring, false)?;
    }
    *scoped.current.lock() = silhouette;
    Ok(())
}
