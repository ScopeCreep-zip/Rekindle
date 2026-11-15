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
//! It is measured per pixel, on the decoded full-canvas frames, at the
//! delays the GIF declares, over its loop (the canonical re-encode loops
//! forever):
//!
//! * a **change** is recorded when a pixel's relative luminance has moved
//!   ≥ 0.10 from the extreme it reached since the last change, in the
//!   opposite direction, with the darker end below 0.80;
//! * a **flash** is a pair of opposing changes, so `n` changes inside a
//!   one-second window `[t, t + 1 s)` are `⌊n / 2⌋` flashes;
//! * the worst pixel and the worst window are the image's count.
//!
//! The definition's area exception (flashes covering ≤ 0.006 sr of a 10°
//! field) is not taken: the viewing distance and the region's rendered
//! size are not the validator's to know, so a single flashing pixel
//! counts. A browser that lengthens very short delays only slows the
//! flashing, so measuring at the declared delays never passes what it
//! renders faster.

use rekindle_image::Frame;

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

/// The most flashes inside any one-second window that starts after the
/// first loop (so the extremes are settled) of a series of change times.
fn worst_window(changes: &[f64], settled: f64) -> u32 {
    let mut worst = 0usize;
    for (i, t) in changes.iter().enumerate().filter(|(_, t)| **t >= settled) {
        let n = changes[i..]
            .iter()
            .take_while(|u| **u < t + WINDOW_MS)
            .count();
        worst = worst.max(n);
    }
    u32::try_from(worst / 2).unwrap_or(u32::MAX)
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

/// The most general flashes any pixel of `frames` shows inside any
/// one-second period of the looping animation.
#[must_use]
pub fn max_flashes(frames: &[Frame]) -> u32 {
    let Some(first) = frames.first() else {
        return 0;
    };
    let mut starts = Vec::with_capacity(frames.len());
    let mut period = 0.0;
    for f in frames {
        starts.push(period);
        period += delay_ms(f);
    }
    if frames.len() < 2 {
        return 0;
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
    let (width, height) = first.buffer().dimensions();
    let mut series = vec![0.0; frames.len()];
    let mut worst = 0;
    for y in 0..height {
        for x in 0..width {
            for (s, f) in series.iter_mut().zip(frames) {
                let p = f.buffer().get_pixel_checked(x, y).map_or([0; 4], |p| p.0);
                *s = colour::luminance([p[0], p[1], p[2]]);
            }
            let found = changes(&series, &starts, period, cycles);
            worst = worst.max(worst_window(&found, period));
            if worst > MAX_FLASHES_PER_SECOND {
                return worst;
            }
        }
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_square_wave_counts_one_flash_per_cycle() {
        // 4 Hz black/white: 8 changes a second, 4 flashes.
        let series = [0.0, 1.0];
        let found = changes(&series, &[0.0, 120.0], 250.0, 6);
        assert_eq!(worst_window(&found, 250.0), 4);
        // A 0.05 wobble is no change.
        let faint = changes(&[0.3, 0.35], &[0.0, 50.0], 100.0, 12);
        assert!(faint.is_empty());
        // Both ends above 0.80: no flash (the darker image is not below).
        let bright = changes(&[0.85, 1.0], &[0.0, 50.0], 100.0, 12);
        assert!(bright.is_empty());
    }
}
