//! The silhouette from the shown skin (plan SK0 S3 on S1): the Host
//! answers `SkinFrames` from the helper-validated package, and each
//! window's shape is `rekindle_skin::shape::silhouette` of its kind's
//! layout at its logical size and scale. Nothing here reads the webview.

use std::collections::BTreeMap;
use std::sync::Arc;

use rekindle_ipc::protocol::IpcRequest;
use rekindle_skin::shape::{silhouette, Mask, SCALE_ONE};
use rekindle_types::dto::skin::{SkinWindow, WindowLayout};
use rekindle_types::dto::skin_library::{SkinApplied, SkinFrames};
use tauri::{AppHandle, Manager, PhysicalSize};

use super::{ShapeRect, Silhouette};
use crate::state::SharedState;
use crate::window_labels::WindowKind;

/// The shown skin's frames, decoded: each window kind's layout and masks.
pub type SkinFrameMap = BTreeMap<SkinWindow, (WindowLayout, Vec<Mask>)>;

/// The skin frame a window kind wears: the buddy list the main frame, a
/// conversation the chat frame, a community window the community frame
/// (plan SKW1: composed from the pack's chat window); every other window
/// keeps its rectangle. The community browser shares the Community kind but
/// wears no frame: [`frame_of_label`] tells them apart.
#[must_use]
pub const fn frame_of(kind: WindowKind) -> Option<SkinWindow> {
    match kind {
        WindowKind::BuddyList => Some(SkinWindow::Main),
        WindowKind::Chat => Some(SkinWindow::Chat),
        WindowKind::Community => Some(SkinWindow::Community),
        WindowKind::Login
        | WindowKind::Settings
        | WindowKind::Call
        | WindowKind::Profile
        | WindowKind::SkinEditor
        | WindowKind::SkinPopup => None,
    }
}

/// The skin frame the window labelled `label` wears: its kind's
/// ([`frame_of`]), except the community browser (`community-browser`, a
/// community window opened with no community), which keeps its rectangle as
/// its webview lays no community tree.
#[must_use]
pub fn frame_of_label(label: &str) -> Option<SkinWindow> {
    if label == crate::window_labels::community_label(None) {
        return None;
    }
    WindowKind::of_label(label).and_then(frame_of)
}

/// The window kind wearing `frame` (the inverse of [`frame_of`]).
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
#[cfg(test)]
pub fn min_window(frames: &SkinFrameMap, frame: Option<SkinWindow>) -> Option<(f64, f64)> {
    let (layout, _) = frame.and_then(|f| frames.get(&f))?;
    let (w, h) = rekindle_skin::shape::min_window(layout)?;
    Some((f64::from(w), f64::from(h)))
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
                .map(|m| Mask::from_parts(m.width, m.height, m.bits, m.faint))
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
    let device_edge = |logical, physical| {
        let nominal = u64::from(logical) * u64::from(per_mille) / u64::from(SCALE_ONE);
        // A rounded logical canvas can end one pixel before the native
        // client area. Keep interior art edges and reconcile the outer edge.
        move |edge: u32| {
            if u64::from(edge) == nominal {
                physical
            } else {
                edge.min(physical)
            }
        }
    };
    let right = device_edge(w, size.width);
    let bottom = device_edge(h, size.height);
    let rects = silhouette(layout, masks, w, h, per_mille).map_err(|e| format!("{e:?}"))?;
    let rects: Vec<ShapeRect> = rects
        .into_iter()
        .map(|r| -> Result<ShapeRect, String> {
            let x = r.x.min(size.width);
            let y = r.y.min(size.height);
            Ok(ShapeRect {
                x: i32::try_from(x).map_err(|e| e.to_string())?,
                y: i32::try_from(y).map_err(|e| e.to_string())?,
                w: right(r.x.saturating_add(r.width)).saturating_sub(x),
                h: bottom(r.y.saturating_add(r.height)).saturating_sub(y),
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

/// Match the frames to the announced stylesheet by its canonical URL.
/// An older fetch or a different frame/stylesheet pair cannot enter native layout.
pub fn refresh(app: &AppHandle, announced: Option<SkinApplied>) {
    let Some(state) = app.try_state::<SharedState>().map(|s| Arc::clone(&s)) else {
        return;
    };
    let generation = super::begin_fetch(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = stage(&app, &state, generation, announced).await {
            tracing::debug!(%error, generation, "skin presentation not staged");
        }
    });
}

/// Resolve and stage the unlocked identity's presentation before building
/// its first framed window. The Host still owns selection and validation.
pub(in crate::windows) async fn prepare(
    app: &AppHandle,
    state: &SharedState,
) -> Result<(), String> {
    stage(app, state, super::begin_fetch(app), None).await
}

async fn stage(
    app: &AppHandle,
    state: &SharedState,
    generation: u64,
    announced: Option<SkinApplied>,
) -> Result<(), String> {
    let skin = if let Some(skin) = announced {
        skin
    } else {
        crate::host_call::call::<SkinApplied>(state, IpcRequest::SkinGet { id: None })
            .await
            .map_err(|e| e.to_string())?
            .value
    };
    let frames = crate::host_call::call::<SkinFrames>(state, IpcRequest::SkinFrames)
        .await
        .map_err(|e| e.to_string())?
        .value;
    if frames.id != skin.id || frames.tokens_href != skin.tokens_href {
        return Err("Host skin frames were superseded during fetch".to_owned());
    }
    super::set_frames(app, generation, decode(frames)?, skin)
}

#[cfg(test)]
#[path = "skin_tests.rs"]
mod tests;
