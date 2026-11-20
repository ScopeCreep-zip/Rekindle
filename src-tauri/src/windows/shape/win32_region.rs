//! Windows shaper: the silhouette becomes the window region, exactly as
//! Xfire did (`Xfire.exe` imports `SetWindowRgn`, `CreateRectRgn`,
//! `CombineRgn`; research §1). "The system does not display any portion of
//! a window that lies outside of the window region", and the window takes
//! no hit there, so clicks reach the window below
//! (learn.microsoft.com `nf-winuser-setwindowrgn`). The region clips the
//! WebView2 child and tao's DWM blur-behind too.
//!
//! FFI through the `windows` crate (0.62.2, already locked through tauri,
//! tao and wry; `Win32/Graphics/Gdi/mod.rs:140,403,463,1966`). No safe
//! wrapper for window regions exists, so each call is an `unsafe` block
//! with its `// SAFETY:` reasoning, under the style guide's FFI rule
//! (precedent `src-tauri/src/platform.rs`).
//!
//! Ownership: every region this module creates is an [`OwnedRegion`],
//! deleted on drop, until `SetWindowRgn` succeeds; from then on "the system
//! owns the region specified by the region handle hRgn. The system does
//! not make a copy of the region. Thus, you should not make any further
//! function calls with this region handle. In particular, do not delete
//! this region handle." (same page), so it is released, never deleted.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, HRGN, RGN_ERROR, RGN_OR,
};

use super::geometry::{win32_corners, Silhouette};

/// A GDI region this module created and still owns.
struct OwnedRegion(HRGN);

impl OwnedRegion {
    /// `CreateRectRgn(left, top, right, bottom)`, right/bottom exclusive.
    fn rect((left, top, right, bottom): (i32, i32, i32, i32)) -> Result<Self, String> {
        // SAFETY: CreateRectRgn takes four integers by value and returns a
        // new region handle, or null on failure; no pointer crosses the
        // boundary and no other state is touched.
        let handle = unsafe { CreateRectRgn(left, top, right, bottom) };
        if handle.is_invalid() {
            return Err("CreateRectRgn failed".to_owned());
        }
        Ok(Self(handle))
    }

    /// `self = self ∪ other` (`CombineRgn(self, self, other, RGN_OR)`).
    fn union(&self, other: &Self) -> Result<(), String> {
        // SAFETY: both handles are live regions created by `rect` and owned
        // here (neither was handed to the system nor deleted); CombineRgn
        // allows the destination to be one of the sources ("hrgnDst … can
        // be one of the source regions"), and writes only into it.
        let kind = unsafe { CombineRgn(Some(self.0), Some(self.0), Some(other.0), RGN_OR) };
        if kind == RGN_ERROR {
            return Err("CombineRgn failed".to_owned());
        }
        Ok(())
    }

    /// Give the handle up to the system after a successful `SetWindowRgn`:
    /// it must not be deleted from here on.
    fn release(self) {
        std::mem::forget(self);
    }
}

impl Drop for OwnedRegion {
    fn drop(&mut self) {
        // SAFETY: the handle was created by `rect`, was never handed to the
        // system (`release` forgets the value instead of dropping it), is
        // not selected into any device context, and is deleted exactly once
        // here.
        let _ = unsafe { DeleteObject(self.0.into()) };
    }
}

/// The silhouette as one region: the first rectangle, then every other
/// one OR'd in, in silhouette order ([`win32_corners`]).
fn build(silhouette: &Silhouette) -> Result<OwnedRegion, String> {
    let corners = win32_corners(silhouette);
    let (first, rest) = corners
        .split_first()
        .ok_or_else(|| "a silhouette has at least one rectangle".to_owned())?;
    let region = OwnedRegion::rect(*first)?;
    for corner in rest {
        region.union(&OwnedRegion::rect(*corner)?)?;
    }
    Ok(region)
}

/// Make `silhouette` the window region of `hwnd`, redrawing it.
///
/// `hwnd` must be the live top-level window, read on the thread that owns
/// it (the caller runs on the event loop's main thread).
pub(super) fn set(hwnd: HWND, silhouette: &Silhouette) -> Result<(), String> {
    let region = build(silhouette)?;
    // SAFETY: `hwnd` is the window handle tauri returned on its owning
    // thread for a window that is open, and `region.0` is a valid region
    // owned here. On success (non-zero) the system owns the region and it
    // is released below, never used or deleted again; on failure it is
    // still ours and is deleted when `region` drops.
    let done = unsafe { SetWindowRgn(hwnd, Some(region.0), true) };
    if done == 0 {
        return Err("SetWindowRgn failed".to_owned());
    }
    region.release();
    Ok(())
}

/// Remove the window region of `hwnd`: the whole window draws and takes
/// input again ("If hRgn is NULL, the function sets the window region to
/// NULL").
pub(super) fn clear(hwnd: HWND) -> Result<(), String> {
    // SAFETY: `hwnd` is a live window handle read on its owning thread
    // (see `set`); a null region passes no handle, so nothing is owned or
    // transferred.
    let done = unsafe { SetWindowRgn(hwnd, None, true) };
    if done == 0 {
        return Err("SetWindowRgn(NULL) failed".to_owned());
    }
    Ok(())
}
