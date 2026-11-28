//! WCAG 2.2 SC 2.3.1, Three Flashes or Below Threshold (plan E7.7.1 step
//! 6): an animated image may not flash more than three times in any one
//! second.
//!
//! The test is WCAG 2.2's definition of the general flash threshold
//! (`.claude/plans/skins/raw/c/specs/wcag22.txt:1513-1522`):
//!
//! > A general flash is defined as a pair of opposing changes in relative
//! > luminance of 10% or more of the maximum relative luminance (1.0) where
//! > the relative luminance of the darker image is below 0.80; and where "a
//! > pair of opposing changes" is an increase followed by a decrease, or a
//! > decrease followed by an increase.
//!
//! and the red flash of [`red`]; the image passes when either count is at
//! most three in every one-second period, or when the pixels over it
//! cover no more than the small area of [`area`] (`wcag22.txt:1513-1530`):
//!
//! > a flash or rapidly changing image sequence is below the threshold
//! > (i.e., content passes) if any of the following are true: there are no
//! > more than three general flashes and / or no more than three red
//! > flashes within any one-second period; or the combined area of flashes
//! > occurring concurrently occupies no more than a total of .006
//! > steradians within any 10 degree visual field on the screen (25% of any
//! > 10 degree visual field on the screen) at typical viewing distance
//!
//! It is measured per pixel, on the decoded full-canvas frames, at the
//! delays the GIF declares, over its loop (the canonical re-encode loops
//! forever):
//!
//! * a **change** is recorded when a pixel's relative luminance has moved
//!   ≥ 0.10 from the extreme it reached since the last change, in the
//!   opposite direction, with the darker end below 0.80;
//! * a **flash** is a pair of opposing changes, so `n` changes inside a
//!   one-second window `[t, t + 1 s)` are `⌊n / 2⌋` flashes;
//! * the windows start at every frame of a settled loop (the first loop
//!   settles the extremes), and in each the pixels with more than three
//!   general or red flashes are the concurrent flashing area, measured for
//!   every way the image is drawn ([`DrawnAs`]).
//!
//! *Not taken:* the exception for "a fine, balanced, pattern such as white
//! noise or an alternating checkerboard pattern with "squares" smaller
//! than 0.1 degree": such a pattern still counts, the stricter reading. A
//! browser that lengthens very short delays only slows the flashing, so
//! measuring at the declared delays never passes what it renders faster.

mod area;
mod red;

use rekindle_image::Frame;
use rekindle_types::dto::skin::{RegionFit, SkinRule};

use crate::colour;

/// SC 2.3.1: at most three flashes in any one-second period.
pub const MAX_FLASHES_PER_SECOND: u32 = 3;

/// A change is ≥ 10 % of the maximum relative luminance (1.0).
const CHANGE: f64 = 0.10;

/// The darker image of a change is below 0.80.
const DARKER_BELOW: f64 = 0.80;

/// One second, in milliseconds.
const WINDOW_MS: f64 = 1000.0;

/// The direction of the last recorded change.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    None,
    Up,
    Down,
}

/// The change times of one luminance series replayed `cycles` times.
fn changes(series: &[f64], starts: &[f64], period: f64, cycles: u32) -> Vec<f64> {
    let mut out = Vec::new();
    let mut dir = Dir::None;
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for cycle in 0..cycles {
        let offset = f64::from(cycle) * period;
        for (v, start) in series.iter().zip(starts) {
            let t = offset + start;
            lo = lo.min(*v);
            hi = hi.max(*v);
            let up = dir != Dir::Up && *v - lo >= CHANGE && lo < DARKER_BELOW;
            let down = dir != Dir::Down && hi - *v >= CHANGE && *v < DARKER_BELOW;
            if up {
                out.push(t);
                dir = Dir::Up;
                (lo, hi) = (*v, *v);
            } else if down {
                out.push(t);
                dir = Dir::Down;
                (lo, hi) = (*v, *v);
            }
        }
    }
    out
}

/// A frame's delay in milliseconds.
fn delay_ms(frame: &Frame) -> f64 {
    let (numer, denom) = frame.delay().numer_denom_ms();
    if denom == 0 {
        0.0
    } else {
        f64::from(numer) / f64::from(denom)
    }
}

/// How one axis of an image is drawn into its box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisFit {
    /// Scaled to the box, which grows with the window.
    Stretch,
    /// Repeated at 1:1 from the start.
    Tile,
    /// `start` and `end` pixels at 1:1, the middle scaled to the box.
    Slice { start: u32, end: u32 },
}

/// How an image is drawn: each axis independently (the region fits of
/// plan E7.7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawnAs {
    pub x: AxisFit,
    pub y: AxisFit,
}

impl DrawnAs {
    /// Scaled to a box of any size on both axes: an image drawn where the
    /// validator does not bound its size (an icon, an entry no region
    /// names, an importer's single file).
    pub const UNBOUNDED: Self = Self {
        x: AxisFit::Stretch,
        y: AxisFit::Stretch,
    };

    /// A region's fit.
    #[must_use]
    pub fn of(fit: &RegionFit) -> Self {
        match fit {
            RegionFit::Stretch => Self::UNBOUNDED,
            RegionFit::Tile => Self {
                x: AxisFit::Tile,
                y: AxisFit::Tile,
            },
            RegionFit::NineSlice(i) => Self {
                x: AxisFit::Slice {
                    start: u32::from(i.left),
                    end: u32::from(i.right),
                },
                y: AxisFit::Slice {
                    start: u32::from(i.top),
                    end: u32::from(i.bottom),
                },
            },
        }
    }
}

/// The loop's frame start times, its period, how many loops are replayed,
/// and the one-second window starts.
struct Timeline {
    starts: Vec<f64>,
    period: f64,
    cycles: u32,
    windows: Vec<f64>,
}

fn timeline(frames: &[Frame]) -> Timeline {
    let mut starts = Vec::with_capacity(frames.len());
    let mut period = 0.0;
    for f in frames {
        starts.push(period);
        period += delay_ms(f);
    }
    if period <= 0.0 {
        // Every frame at once: any change is infinitely fast.
        period = f64::MIN_POSITIVE;
    }
    // One loop to settle the extremes, then enough loops to cover a full
    // window starting anywhere in the second.
    let span = (WINDOW_MS / period).ceil().min(f64::from(u16::MAX));
    let cycles = 2 + format!("{span:.0}")
        .parse::<u32>()
        .unwrap_or(u32::from(u16::MAX));
    // Every frame of the first settled loop, and the next loop's start: the
    // replay runs ≥ 1 s past each (a loop's pattern repeats from there).
    let mut windows: Vec<f64> = starts.iter().map(|s| period + s).collect();
    windows.push(2.0 * period);
    Timeline {
        starts,
        period,
        cycles,
        windows,
    }
}

/// Flashes (pairs of changes) of `changes` inside `[t, t + 1 s)`.
fn flashes_at(changes: &[f64], t: f64) -> u32 {
    let a = changes.partition_point(|u| *u < t);
    let b = changes.partition_point(|u| *u < t + WINDOW_MS);
    u32::try_from((b - a) / 2).unwrap_or(u32::MAX)
}

/// One window start: the pixels over the threshold and the worst counts.
struct Window {
    flagged: Vec<bool>,
    general: u32,
    red: u32,
}

/// WCAG 2.3.1 on the frames of an animated image drawn every way in
/// `drawn_as`: `Ok` when every pixel flashes at most three times a
/// second, or the pixels that flash more cover no more than the small
/// area of every drawn_as's fields at once.
///
/// # Errors
/// [`SkinRule::Flashing`] (general) or [`SkinRule::RedFlashing`], with the
/// worst count of the first window whose area is too large.
pub fn check(frames: &[Frame], drawn_as: &[DrawnAs]) -> Result<(), SkinRule> {
    let Some(first) = frames.first() else {
        return Ok(());
    };
    if frames.len() < 2 {
        return Ok(());
    }
    let tl = timeline(frames);
    let (width, height) = first.buffer().dimensions();
    let (w, h) = (width as usize, height as usize);
    let mut windows: Vec<Window> = tl
        .windows
        .iter()
        .map(|_| Window {
            flagged: vec![false; w * h],
            general: 0,
            red: 0,
        })
        .collect();
    let mut lum = vec![0.0; frames.len()];
    let mut chr = vec![red::chroma([0, 0, 0]); frames.len()];
    let mut over = false;
    for y in 0..height {
        for x in 0..width {
            for ((l, c), f) in lum.iter_mut().zip(chr.iter_mut()).zip(frames) {
                let p = f.buffer().get_pixel_checked(x, y).map_or([0; 4], |p| p.0);
                *l = colour::luminance([p[0], p[1], p[2]]);
                *c = red::chroma([p[0], p[1], p[2]]);
            }
            let general = changes(&lum, &tl.starts, tl.period, tl.cycles);
            let reds = red::changes(&chr, &tl.starts, tl.period, tl.cycles);
            for (win, t) in windows.iter_mut().zip(&tl.windows) {
                let (g, r) = (flashes_at(&general, *t), flashes_at(&reds, *t));
                if g > MAX_FLASHES_PER_SECOND || r > MAX_FLASHES_PER_SECOND {
                    win.flagged[y as usize * w + x as usize] = true;
                    win.general = win.general.max(g);
                    win.red = win.red.max(r);
                    over = true;
                }
            }
        }
    }
    if !over {
        return Ok(());
    }
    for win in &windows {
        if win.general.max(win.red) <= MAX_FLASHES_PER_SECOND {
            continue;
        }
        if drawn_as
            .iter()
            .any(|p| area::exceeds(w, h, &win.flagged, *p))
        {
            let max = MAX_FLASHES_PER_SECOND;
            return Err(if win.general > max {
                SkinRule::Flashing {
                    flashes: win.general,
                    max,
                }
            } else {
                SkinRule::RedFlashing {
                    flashes: win.red,
                    max,
                }
            });
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
