//! Color tier detection and 24-bit → 256 → 16 degradation.
//!
//! Every color decision accounts for terminal capability. Colors are
//! generated at theme load time for the detected tier — no runtime
//! branching in the render path.

use ratatui::style::Color;

/// Detected color capability tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTier {
    /// 24-bit true color (16.7M colors). iTerm2, kitty, Alacritty, Windows Terminal.
    TrueColor,
    /// 256 indexed colors. xterm-256color and above.
    Color256,
    /// Basic 16 ANSI colors. Real TTYs, TERM=dumb, minimal terminals.
    Color16,
}

impl ColorTier {
    /// Detect color capability from environment.
    pub fn detect() -> Self {
        if let Ok(ct) = std::env::var("COLORTERM") {
            if ct == "truecolor" || ct == "24bit" {
                return Self::TrueColor;
            }
        }
        if let Ok(term) = std::env::var("TERM") {
            if term.contains("256color") {
                return Self::Color256;
            }
            if term.starts_with("xterm") || term.starts_with("screen") || term.starts_with("tmux") {
                return Self::Color256;
            }
        }
        Self::Color16
    }

    /// Whether true-color RGB is available.
    pub fn has_true_color(self) -> bool {
        self == Self::TrueColor
    }

    /// Whether 256-color palette is available.
    pub fn has_256(self) -> bool {
        matches!(self, Self::TrueColor | Self::Color256)
    }
}

/// Convert an RGB tuple to the appropriate Color for the detected tier.
pub fn degrade(rgb: (u8, u8, u8), tier: ColorTier) -> Color {
    match tier {
        ColorTier::TrueColor => Color::Rgb(rgb.0, rgb.1, rgb.2),
        ColorTier::Color256 => Color::Indexed(rgb_to_256(rgb.0, rgb.1, rgb.2)),
        ColorTier::Color16 => rgb_to_16(rgb.0, rgb.1, rgb.2),
    }
}

/// Convert 24-bit RGB to 256-color index.
///
/// Uses btop's algorithm: greyscale ramp for near-grey colors,
/// 6×6×6 color cube for everything else.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    let r_q = (f64::from(r) / 11.0).round() as u8;
    let g_q = (f64::from(g) / 11.0).round() as u8;
    let b_q = (f64::from(b) / 11.0).round() as u8;

    if r_q == g_q && g_q == b_q {
        // Greyscale ramp (232-255, 24 shades)
        let grey = 232 + r_q.min(23);
        return grey;
    }

    // 6×6×6 color cube (16-231)
    let r6 = (f64::from(r) / 51.0).round() as u8;
    let g6 = (f64::from(g) / 51.0).round() as u8;
    let b6 = (f64::from(b) / 51.0).round() as u8;
    16 + 36 * r6 + 6 * g6 + b6
}

/// Convert 24-bit RGB to the nearest basic 16 ANSI color.
fn rgb_to_16(r: u8, g: u8, b: u8) -> Color {
    let luma = (u16::from(r) * 299 + u16::from(g) * 587 + u16::from(b) * 114) / 1000;
    let is_bright = luma > 128;

    // Dominant channel determines hue
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let saturation = if max == 0 { 0 } else { (u16::from(max - min) * 255) / u16::from(max) };

    if saturation < 30 {
        // Near-grey
        return if luma > 192 {
            Color::White
        } else if luma > 96 {
            Color::Gray
        } else if luma > 32 {
            Color::DarkGray
        } else {
            Color::Black
        };
    }

    // Determine hue from dominant channel
    let hue_color = if r >= g && r >= b {
        if g > b + 30 { Color::Yellow } else { Color::Red }
    } else if g >= r && g >= b {
        if b > r + 30 { Color::Cyan } else { Color::Green }
    } else if r > g + 30 {
        Color::Magenta
    } else {
        Color::Blue
    };

    // Apply brightness
    if is_bright {
        match hue_color {
            Color::Red => Color::LightRed,
            Color::Green => Color::LightGreen,
            Color::Yellow => Color::LightYellow,
            Color::Blue => Color::LightBlue,
            Color::Magenta => Color::LightMagenta,
            Color::Cyan => Color::LightCyan,
            other => other,
        }
    } else {
        hue_color
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_valid_tier() {
        let tier = ColorTier::detect();
        // Should be one of the three valid tiers
        assert!(matches!(tier, ColorTier::TrueColor | ColorTier::Color256 | ColorTier::Color16));
    }

    #[test]
    fn degrade_true_color_passthrough() {
        let c = degrade((0xca, 0x9e, 0xe6), ColorTier::TrueColor);
        assert_eq!(c, Color::Rgb(0xca, 0x9e, 0xe6));
    }

    #[test]
    fn degrade_256_produces_indexed() {
        let c = degrade((255, 0, 0), ColorTier::Color256);
        assert!(matches!(c, Color::Indexed(_)));
    }

    #[test]
    fn degrade_16_produces_named() {
        let c = degrade((255, 0, 0), ColorTier::Color16);
        assert!(matches!(c, Color::Red | Color::LightRed));
    }

    #[test]
    fn rgb_to_256_greyscale() {
        // Pure grey should map to greyscale ramp
        let idx = rgb_to_256(128, 128, 128);
        assert!((232..=255).contains(&idx), "grey should be in 232-255 range, got {idx}");
    }

    #[test]
    fn rgb_to_256_pure_red() {
        let idx = rgb_to_256(255, 0, 0);
        // Should be in the color cube: 16 + 36*5 + 6*0 + 0 = 196
        assert_eq!(idx, 196);
    }

    #[test]
    fn rgb_to_16_black() {
        assert_eq!(rgb_to_16(0, 0, 0), Color::Black);
    }

    #[test]
    fn rgb_to_16_white() {
        assert_eq!(rgb_to_16(255, 255, 255), Color::White);
    }
}
