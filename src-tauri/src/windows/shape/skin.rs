//! The silhouette from the shown skin (plan SK0 S3 on S1): the Host
//! answers `SkinFrames` from the helper-validated package, and each
//! window's shape is `rekindle_skin::shape::silhouette` of its kind's
//! layout at its logical size and scale. Nothing here reads the webview.

use std::collections::BTreeMap;
use std::sync::Arc;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_skin::shape::{silhouette, Mask, SCALE_ONE};
use rekindle_types::dto::skin::{SkinWindow, WindowLayout};
use rekindle_types::dto::skin_library::SkinFrames;
use tauri::{AppHandle, Manager, PhysicalSize};

use super::{ShapeRect, Silhouette};
use crate::state::SharedState;
use crate::window_labels::WindowKind;

/// The shown skin's frames, decoded: each window kind's layout and masks.
pub type SkinFrameMap = BTreeMap<SkinWindow, (WindowLayout, Vec<Mask>)>;

/// The skin frame a window kind wears: the buddy list the main frame, a
/// conversation the chat frame; every other window keeps its rectangle.
#[must_use]
pub const fn frame_of(kind: WindowKind) -> Option<SkinWindow> {
    match kind {
        WindowKind::BuddyList => Some(SkinWindow::Main),
        WindowKind::Chat => Some(SkinWindow::Chat),
        WindowKind::Login
        | WindowKind::Settings
        | WindowKind::Community
        | WindowKind::Call
        | WindowKind::Profile
        | WindowKind::SkinEditor
        | WindowKind::SkinPopup => None,
    }
}

/// The window kind wearing `frame` (the inverse of [`frame_of`]; the
/// Community frame's kind is the Community window, which [`frame_of`]
/// does not yet dress: no pack serves that layout before plan SK17-6).
#[must_use]
pub const fn kind_of(frame: SkinWindow) -> WindowKind {
    match frame {
        SkinWindow::Main => WindowKind::BuddyList,
        SkinWindow::Chat => WindowKind::Chat,
        SkinWindow::Community => WindowKind::Community,
    }
}

/// The smallest window (logical px) that holds `frame` of `frames`: the
/// skin crate's [`rekindle_skin::shape::min_window`] of its served layout,
/// the same value the Host serves on the window's shell (plan SK4). `None`
/// when the skin lays no frame for that kind (or a minimum past `u16`,
/// which the helper never passes).
#[must_use]
pub fn min_window(frames: &SkinFrameMap, frame: Option<SkinWindow>) -> Option<(f64, f64)> {
    let (layout, _) = frame.and_then(|f| frames.get(&f))?;
    let (w, h) = rekindle_skin::shape::min_window(layout)?;
    Some((f64::from(w), f64::from(h)))
}

/// The size (logical px) to grow a window of `current` to so it holds
/// `min`; `None` when it already does. A larger window is never shrunk.
#[must_use]
pub fn grown(current: (f64, f64), min: (f64, f64)) -> Option<(f64, f64)> {
    if current.0 >= min.0 && current.1 >= min.1 {
        return None;
    }
    Some((current.0.max(min.0), current.1.max(min.1)))
}

/// Decode the Host's answer (tile masks, then outline node art masks); a
/// mask whose bits do not match its size
/// refuses the whole answer.
///
/// # Errors
/// A mask is malformed.
pub fn decode(frames: SkinFrames) -> Result<SkinFrameMap, String> {
    frames
        .windows
        .into_iter()
        .map(|w| {
            let masks = w
                .masks
                .into_iter()
                .map(|m| Mask::from_bits(m.width, m.height, m.bits))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| format!("malformed mask in {:?}", w.window))?;
            Ok((w.window, (w.layout, masks)))
        })
        .collect()
}

/// The silhouette of a `size` (device px) window at `scale` wearing
/// `frame` of `frames`: the skin's opaque region, or the whole window when
/// the skin has no layout for that kind.
///
/// # Errors
/// A zero or unrepresentable size, or a layout the shaper refuses.
pub fn compute(
    frames: &SkinFrameMap,
    frame: Option<SkinWindow>,
    size: PhysicalSize<u32>,
    scale: f64,
) -> Result<Silhouette, String> {
    let Some((layout, masks)) = frame.and_then(|f| frames.get(&f)) else {
        return Silhouette::full(size.width, size.height);
    };
    let per_mille = per_mille(scale).ok_or_else(|| format!("scale {scale} is out of range"))?;
    let logical = |px: u32| {
        u32::try_from(
            (u64::from(px) * u64::from(SCALE_ONE) + u64::from(per_mille / 2))
                / u64::from(per_mille),
        )
        .map_err(|e| e.to_string())
    };
    let (w, h) = (logical(size.width)?, logical(size.height)?);
    let rects = silhouette(layout, masks, w, h, per_mille).map_err(|e| format!("{e:?}"))?;
    let rects: Vec<ShapeRect> = rects
        .into_iter()
        .map(|r| -> Result<ShapeRect, String> {
            Ok(ShapeRect {
                x: i32::try_from(r.x).map_err(|e| e.to_string())?,
                y: i32::try_from(r.y).map_err(|e| e.to_string())?,
                w: r.width,
                h: r.height,
            })
        })
        .collect::<Result<_, _>>()?;
    Silhouette::new(&rects)
}

/// `scale` in thousandths, rounded to nearest (integer search, no float
/// cast); `None` outside 0.001..=100.
fn per_mille(scale: f64) -> Option<u32> {
    let target = scale * f64::from(SCALE_ONE);
    if !(target.is_finite() && (0.5..=100_000.0).contains(&target)) {
        return None;
    }
    // The least p with p + 0.5 > target: target rounded half up.
    let (mut lo, mut hi) = (1u32, 100_000u32);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if f64::from(mid) + 0.5 > target {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Some(lo)
}

/// Fetch the shown skin's frames from the Host, keep them, and reshape
/// every window (on the first shaped window and on every skin change); an
/// answer a later change superseded is dropped.
pub fn refresh(app: &AppHandle) {
    let Some(state) = app.try_state::<SharedState>().map(|s| Arc::clone(&s)) else {
        return;
    };
    let generation = super::begin_fetch(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let answer = crate::host_call::call::<SkinFrames>(&state, IpcRequest::SkinFrames).await;
        let frames = match answer
            .map_err(|e| format!("{e:?}"))
            .and_then(|d| decode(d.value))
        {
            Ok(frames) => frames,
            Err(e) => {
                tracing::warn!(error = %e, "skin frames unavailable; windows stay whole");
                SkinFrameMap::new()
            }
        };
        super::set_frames(&app, generation, frames);
    });
}

#[cfg(test)]
#[path = "skin_tests.rs"]
mod tests;
