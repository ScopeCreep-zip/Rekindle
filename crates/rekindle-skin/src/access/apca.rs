//! APCA-W3 0.0.98G-4g lightness contrast (Lc), the editor's advisory
//! figure beside each pair's WCAG 2.x ratio (plan E7.7.5: "APCA
//! advisory"; `.claude/plans/skins/01-recommendation.md` §7). WCAG 2.x
//! stays the gate. Constants from the published APCA-W3 base algorithm
//! (Myndex `apca-w3` 0.1.9, `SA98G`).

const MAIN_TRC: f64 = 2.4;
const COEF: [f64; 3] = [0.212_672_9, 0.715_152_2, 0.072_175_0];
const NORM_BG: f64 = 0.56;
const NORM_TXT: f64 = 0.57;
const REV_TXT: f64 = 0.62;
const REV_BG: f64 = 0.65;
const BLK_THRS: f64 = 0.022;
const BLK_CLMP: f64 = 1.414;
const SCALE: f64 = 1.14;
const LO_OFFSET: f64 = 0.027;
const LO_CLIP: f64 = 0.1;
const DELTA_Y_MIN: f64 = 0.0005;

/// The screen luminance APCA estimates (a simple 2.4 power, not the sRGB
/// piecewise curve), soft-clamped near black.
fn y(rgb: [u8; 3]) -> f64 {
    let raw: f64 = rgb
        .iter()
        .zip(COEF)
        .map(|(c, k)| (f64::from(*c) / 255.0).powf(MAIN_TRC) * k)
        .sum();
    if raw < BLK_THRS {
        raw + (BLK_THRS - raw).powf(BLK_CLMP)
    } else {
        raw
    }
}

/// Lc of `text` on `ground`: positive for dark text on a light ground,
/// negative for light text on a dark one; 0 below the clip.
#[must_use]
pub fn lc(text: [u8; 3], ground: [u8; 3]) -> f64 {
    let (yt, yb) = (y(text), y(ground));
    if (yb - yt).abs() < DELTA_Y_MIN {
        return 0.0;
    }
    let out = if yb > yt {
        let sapc = (yb.powf(NORM_BG) - yt.powf(NORM_TXT)) * SCALE;
        if sapc < LO_CLIP {
            0.0
        } else {
            sapc - LO_OFFSET
        }
    } else {
        let sapc = (yb.powf(REV_BG) - yt.powf(REV_TXT)) * SCALE;
        if sapc > -LO_CLIP {
            0.0
        } else {
            sapc + LO_OFFSET
        }
    };
    out * 100.0
}

#[cfg(test)]
#[path = "apca_tests.rs"]
mod tests;
