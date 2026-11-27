//! The red flash of WCAG 2.2's "general flash and red flash thresholds"
//! (`.claude/plans/skins/raw/c/specs/wcag22.txt:1522-1537`):
//!
//! > A red flash is defined as any pair of opposing transitions involving a
//! > saturated red
//!
//! > Note 3: The new working definition in the field for "pair of opposing
//! > transitions involving a saturated red" (from WCAG 2.2) is a pair of
//! > opposing transitions where, one transition is either to or from a
//! > state with a value R/(R + G + B) that is greater than or equal to 0.8,
//! > and the difference between states is more than 0.2 (unitless) in the
//! > CIE 1976 UCS chromaticity diagram. [ISO_9241-391]
//!
//! Note 2 places a transition "between adjacent peaks and valleys in a
//! plot of relative luminance/color", so the series is replayed like the
//! luminance one: the plot is the redness `R/(R+G+B)`, its peak and valley
//! since the last transition are kept, and a transition is recorded when
//! the current state is saturated red and lies more than 0.2 from the
//! valley in u′v′ (towards red), or the peak is saturated red and the
//! current state lies more than 0.2 from it (away from red). Transitions
//! alternate, so a flash is a pair of them.
//!
//! *Decided:* the note does not name the encoding of R, G and B. The ratio
//! is taken on linear-light sRGB, which flags more colours as saturated
//! than the gamma-encoded values (`#ff3c3c` is 0.92 linear, 0.68 encoded),
//! the conservative reading for the users the validator protects. u′v′ is
//! CIE 1976 UCS from linear sRGB through the D65 XYZ matrix (IEC
//! 61966-2-1); a colour without chromaticity (black) sits at the D65 white
//! point, so red against black is a red transition as it is against white.

use crate::colour::linear;

/// "R/(R + G + B) that is greater than or equal to 0.8".
const SATURATED: f64 = 0.8;

/// "the difference between states is more than 0.2" in u′v′.
const DELTA: f64 = 0.2;

/// D65 in u′v′, where an achromatic state sits.
const WHITE: (f64, f64) = (0.1978, 0.4683);

/// One pixel state: its redness and its u′v′ chromaticity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Chroma {
    ratio: f64,
    u: f64,
    v: f64,
}

/// The state of an sRGB colour.
pub(super) fn chroma(rgb: [u8; 3]) -> Chroma {
    let [red, green, blue] = rgb.map(linear);
    let sum = red + green + blue;
    let ratio = if sum > 0.0 { red / sum } else { 0.0 };
    // IEC 61966-2-1 linear sRGB to D65 XYZ.
    let cx = 0.4124 * red + 0.3576 * green + 0.1805 * blue;
    let cy = 0.2126 * red + 0.7152 * green + 0.0722 * blue;
    let cz = 0.0193 * red + 0.1192 * green + 0.9505 * blue;
    let denom = cx + 15.0 * cy + 3.0 * cz;
    let (u, v) = if denom > 0.0 {
        (4.0 * cx / denom, 9.0 * cy / denom)
    } else {
        WHITE
    };
    Chroma { ratio, u, v }
}

fn apart(a: Chroma, b: Chroma) -> bool {
    (a.u - b.u).hypot(a.v - b.v) > DELTA
}

/// The red transition times of one state series replayed `cycles` times.
pub(super) fn changes(series: &[Chroma], starts: &[f64], period: f64, cycles: u32) -> Vec<f64> {
    let mut out = Vec::new();
    // `Some(true)`: the last transition went to red; `Some(false)`: away.
    let mut last: Option<bool> = None;
    let Some(first) = series.first() else {
        return out;
    };
    let (mut valley, mut peak) = (*first, *first);
    for cycle in 0..cycles {
        let offset = f64::from(cycle) * period;
        for (c, start) in series.iter().zip(starts) {
            if c.ratio < valley.ratio {
                valley = *c;
            }
            if c.ratio > peak.ratio {
                peak = *c;
            }
            let to_red = last != Some(true) && c.ratio >= SATURATED && apart(*c, valley);
            let from_red = last != Some(false) && peak.ratio >= SATURATED && apart(*c, peak);
            if to_red || from_red {
                out.push(offset + start);
                last = Some(to_red);
                (valley, peak) = (*c, *c);
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "red_tests.rs"]
mod tests;
