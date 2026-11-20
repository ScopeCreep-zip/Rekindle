//! The silhouette's geometry, free of any window system: the rectangles a
//! window keeps, their validation, and the conversions each platform
//! shaper needs (plan SK0 S3). Everything here is pure, so it is tested on
//! every host whatever platform the shaper itself runs on.

/// The most rectangles one silhouette may hold (plan SK0 S4, research
/// §6.4): a hostile skin must not make region building or hit-testing
/// expensive. The skin helper enforces the same cap on the package.
pub const MAX_RECTS: usize = 4096;

/// One rectangle of a window's silhouette, in device (physical) pixels
/// relative to the window's top-left corner. `w` and `h` are exclusive
/// extents: the rectangle covers `x..x + w` by `y..y + h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapeRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl ShapeRect {
    /// The right edge, exclusive, saturating at `i32::MAX`.
    fn right(self) -> i32 {
        self.x
            .saturating_add(i32::try_from(self.w).unwrap_or(i32::MAX))
    }

    /// The bottom edge, exclusive, saturating at `i32::MAX`.
    fn bottom(self) -> i32 {
        self.y
            .saturating_add(i32::try_from(self.h).unwrap_or(i32::MAX))
    }

    const fn is_empty(self) -> bool {
        self.w == 0 || self.h == 0
    }
}

/// A validated silhouette: at least one non-empty rectangle and at most
/// [`MAX_RECTS`]. An empty silhouette would make the window invisible and
/// click-through everywhere (research §6.3), so it is refused rather than
/// applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Silhouette(Vec<ShapeRect>);

impl Silhouette {
    /// Validate `rects`, dropping empty ones.
    ///
    /// # Errors
    /// No non-empty rectangle is left, or more than [`MAX_RECTS`] are given.
    pub fn new(rects: &[ShapeRect]) -> Result<Self, String> {
        if rects.len() > MAX_RECTS {
            return Err(format!(
                "a silhouette of {} rectangles exceeds the cap of {MAX_RECTS}",
                rects.len()
            ));
        }
        let kept: Vec<ShapeRect> = rects.iter().copied().filter(|r| !r.is_empty()).collect();
        if kept.is_empty() {
            return Err("a silhouette needs at least one non-empty rectangle".to_owned());
        }
        Ok(Self(kept))
    }

    /// The whole `width` × `height` window: today's shape, used while no
    /// skin layout gives one.
    ///
    /// # Errors
    /// The window has no area yet.
    pub fn full(width: u32, height: u32) -> Result<Self, String> {
        Self::new(&[ShapeRect {
            x: 0,
            y: 0,
            w: width,
            h: height,
        }])
    }

    pub fn rects(&self) -> &[ShapeRect] {
        &self.0
    }

    /// Whether the device-pixel point `(x, y)` lies inside the silhouette.
    #[cfg(any(target_os = "macos", test))]
    pub fn contains(&self, x: f64, y: f64) -> bool {
        self.0.iter().any(|r| {
            x >= f64::from(r.x)
                && y >= f64::from(r.y)
                && x < f64::from(r.right())
                && y < f64::from(r.bottom())
        })
    }
}

#[cfg(any(target_os = "linux", test))]
/// A rectangle in a toolkit's logical coordinates, `(x, y, width, height)`.
pub type LogicalRect = (i32, i32, i32, i32);

#[cfg(any(target_os = "linux", test))]
/// The silhouette in logical coordinates for GDK, whose window regions are
/// surface-local and device-independent (GTK 3.24
/// `gdk_wayland_window_sync_input_region` hands the region to
/// `wl_surface_set_input_region` unscaled). GTK 3 scales windows by an
/// integer factor only (`gdk_window_get_scale_factor` returns `int`;
/// fractional output scales are the compositor's downscale of that), so the
/// conversion is integer division. Each rectangle is rounded outward, so no
/// pixel of frame art loses its input.
pub fn to_logical(silhouette: &Silhouette, scale: i32) -> Vec<LogicalRect> {
    let scale = scale.max(1);
    silhouette
        .rects()
        .iter()
        .map(|r| {
            let x0 = r.x.div_euclid(scale);
            let y0 = r.y.div_euclid(scale);
            let x1 = ceil_div(r.right(), scale);
            let y1 = ceil_div(r.bottom(), scale);
            (x0, y0, x1 - x0, y1 - y0)
        })
        .collect()
}

#[cfg(any(target_os = "linux", test))]
/// `value / divisor` rounded toward positive infinity (`divisor > 0`).
fn ceil_div(value: i32, divisor: i32) -> i32 {
    let quotient = value.div_euclid(divisor);
    if value.rem_euclid(divisor) == 0 {
        quotient
    } else {
        quotient + 1
    }
}

#[cfg(any(windows, test))]
/// The Win32 region build order: `CreateRectRgn` takes the first entry's
/// corners `(left, top, right, bottom)`, right and bottom exclusive, and
/// each further entry is `CombineRgn(.., RGN_OR)`ed into it, in silhouette
/// order. Coordinates are window-relative device pixels, as `SetWindowRgn`
/// wants them.
pub fn win32_corners(silhouette: &Silhouette) -> Vec<(i32, i32, i32, i32)> {
    silhouette
        .rects()
        .iter()
        .map(|r| (r.x, r.y, r.right(), r.bottom()))
        .collect()
}

#[cfg(any(target_os = "macos", test))]
/// The cursor relative to the window, in the window's device pixels.
///
/// tao 0.37.1 reports the cursor in physical pixels at the **primary**
/// monitor's scale (`platform_impl/macos/util/mod.rs:101-106`) and the
/// window's position at the **window's** scale (`macos/window.rs:714-724`),
/// so both are brought to logical points first, subtracted there, and the
/// result scaled back to the window's pixels, which the silhouette uses.
pub fn local_point(
    cursor_physical: (f64, f64),
    primary_scale: f64,
    window_physical: (i32, i32),
    window_scale: f64,
) -> (f64, f64) {
    let cursor = (
        cursor_physical.0 / primary_scale,
        cursor_physical.1 / primary_scale,
    );
    let window = (
        f64::from(window_physical.0) / window_scale,
        f64::from(window_physical.1) / window_scale,
    );
    (
        (cursor.0 - window.0) * window_scale,
        (cursor.1 - window.1) * window_scale,
    )
}

#[cfg(test)]
#[path = "geometry_tests.rs"]
mod tests;
