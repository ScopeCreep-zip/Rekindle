//! The terminal's colour depth (plan E7.7.2, the TUI palette): a skin's
//! colours are sRGB, sent as 24-bit where the terminal says it renders
//! them and as the nearest xterm 256-colour index elsewhere.

use ratatui::style::{Color, Style};

/// What the terminal renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourDepth {
    /// 24-bit (`COLORTERM=truecolor` or `24bit`, the convention terminals
    /// that render direct colour advertise).
    TrueColour,
    /// The xterm 256-colour palette.
    Ansi256,
}

/// The xterm colour cube's six levels (indices 16–231).
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

impl ColourDepth {
    /// From `COLORTERM`.
    #[must_use]
    pub fn detect() -> Self {
        Self::of(std::env::var("COLORTERM").ok().as_deref())
    }

    /// From a `COLORTERM` value.
    #[must_use]
    pub fn of(colorterm: Option<&str>) -> Self {
        match colorterm {
            Some("truecolor" | "24bit") => Self::TrueColour,
            _ => Self::Ansi256,
        }
    }

    /// `c` as this depth renders it: an RGB colour becomes its nearest
    /// 256-colour index on `Ansi256`; anything else is unchanged.
    #[must_use]
    pub fn fit(self, c: Color) -> Color {
        match (self, c) {
            (Self::Ansi256, Color::Rgb(r, g, b)) => Color::Indexed(nearest_256([r, g, b])),
            _ => c,
        }
    }

    /// `s` with its foreground, background and underline colours fitted.
    #[must_use]
    pub fn fit_style(self, mut s: Style) -> Style {
        s.fg = s.fg.map(|c| self.fit(c));
        s.bg = s.bg.map(|c| self.fit(c));
        s.underline_color = s.underline_color.map(|c| self.fit(c));
        s
    }
}

fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| u32::from(x.abs_diff(y)).pow(2))
        .sum()
}

/// The nearest cube level's index to `v`.
fn cube_step(v: u8) -> u8 {
    let mut best = 0;
    for (i, level) in (0u8..).zip(CUBE) {
        if level.abs_diff(v) < CUBE[usize::from(best)].abs_diff(v) {
            best = i;
        }
    }
    best
}

/// The xterm 256-colour index nearest to `rgb`: the closer of the colour
/// cube's nearest point and the grey ramp's (232–255, `8 + 10 i`).
#[must_use]
pub fn nearest_256(rgb: [u8; 3]) -> u8 {
    let [r, g, b] = rgb.map(cube_step);
    let cube = [
        CUBE[usize::from(r)],
        CUBE[usize::from(g)],
        CUBE[usize::from(b)],
    ];
    let cube_index = 16 + 36 * r + 6 * g + b;
    let mean = (u16::from(rgb[0]) + u16::from(rgb[1]) + u16::from(rgb[2])) / 3;
    let step = u8::try_from(mean.saturating_sub(3) / 10).map_or(23, |s| s.min(23));
    let grey = 8 + 10 * step;
    if distance([grey; 3], rgb) < distance(cube, rgb) {
        232 + step
    } else {
        cube_index
    }
}

#[cfg(test)]
#[path = "colour_depth_tests.rs"]
mod tests;
