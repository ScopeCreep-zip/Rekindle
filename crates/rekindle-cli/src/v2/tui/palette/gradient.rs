//! Pre-computed 101-step color gradients for data visualization.
//!
//! Every gradient is computed once at theme load time. At render time,
//! looking up a color is O(1) array access: `gradient.at(percentage)`.

use ratatui::style::Color;

/// A pre-computed color gradient with 101 steps (0-100 inclusive).
///
/// Used for: meters, graphs, temperature indicators, process CPU bars,
/// network throughput, memory pressure — any value that maps to a
/// 0-100% range and should show a smooth color transition.
#[derive(Clone)]
pub struct Gradient {
    colors: [Color; 101],
}

#[allow(clippy::needless_range_loop, clippy::cast_precision_loss)]
impl Gradient {
    /// Create a gradient between two colors (linear interpolation).
    pub fn two_color(start: (u8, u8, u8), end: (u8, u8, u8)) -> Self {
        let mut colors = [Color::Reset; 101];
        for i in 0..=100 {
            let t = i as f64 / 100.0;
            let r = lerp(start.0, end.0, t);
            let g = lerp(start.1, end.1, t);
            let b = lerp(start.2, end.2, t);
            colors[i] = Color::Rgb(r, g, b);
        }
        Self { colors }
    }

    /// Create a gradient through three colors (start → mid at 50% → end at 100%).
    pub fn three_color(start: (u8, u8, u8), mid: (u8, u8, u8), end: (u8, u8, u8)) -> Self {
        let mut colors = [Color::Reset; 101];
        for i in 0..=100 {
            let (from, to, t) = if i <= 50 {
                (start, mid, i as f64 / 50.0)
            } else {
                (mid, end, (i - 50) as f64 / 50.0)
            };
            let r = lerp(from.0, to.0, t);
            let g = lerp(from.1, to.1, t);
            let b = lerp(from.2, to.2, t);
            colors[i] = Color::Rgb(r, g, b);
        }
        Self { colors }
    }

    /// Create a flat gradient (single color) — for TTY fallback or non-gradient widgets.
    pub fn flat(color: (u8, u8, u8)) -> Self {
        let c = Color::Rgb(color.0, color.1, color.2);
        Self { colors: [c; 101] }
    }

    /// TTY fallback: 3 flat sections using basic ANSI colors.
    pub fn tty_three_step(low: Color, mid: Color, high: Color) -> Self {
        let mut colors = [Color::Reset; 101];
        for i in 0..=100 {
            colors[i] = match i {
                0..=33 => low,
                34..=66 => mid,
                _ => high,
            };
        }
        Self { colors }
    }

    /// TTY fallback: 2 flat sections.
    pub fn tty_two_step(low: Color, high: Color) -> Self {
        let mut colors = [Color::Reset; 101];
        for i in 0..=100 {
            colors[i] = if i <= 50 { low } else { high };
        }
        Self { colors }
    }

    /// Get the color at a given percentage (0-100, clamped).
    pub fn at(&self, pct: usize) -> Color {
        self.colors[pct.min(100)]
    }

    /// Get the raw RGB at a given percentage. Returns (0,0,0) for non-RGB colors.
    pub fn rgb_at(&self, pct: usize) -> (u8, u8, u8) {
        match self.colors[pct.min(100)] {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => (0, 0, 0),
        }
    }
}

impl std::fmt::Debug for Gradient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Gradient[{:?} → {:?}]", self.colors[0], self.colors[100])
    }
}

/// Linear interpolation between two u8 values.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn lerp(a: u8, b: u8, t: f64) -> u8 {
    (f64::from(a) + t * (f64::from(b) - f64::from(a))).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_color_endpoints() {
        let g = Gradient::two_color((0, 0, 0), (255, 255, 255));
        assert_eq!(g.rgb_at(0), (0, 0, 0));
        assert_eq!(g.rgb_at(100), (255, 255, 255));
    }

    #[test]
    fn two_color_midpoint() {
        let g = Gradient::two_color((0, 0, 0), (200, 200, 200));
        let (r, g_val, b) = g.rgb_at(50);
        assert!((95..=105).contains(&r.into()), "midpoint r={r}");
        assert!((95..=105).contains(&g_val.into()), "midpoint g={g_val}");
        assert!((95..=105).contains(&b.into()), "midpoint b={b}");
    }

    #[test]
    fn three_color_transitions() {
        let g = Gradient::three_color((255, 0, 0), (0, 255, 0), (0, 0, 255));
        // At 0%: red
        assert_eq!(g.rgb_at(0), (255, 0, 0));
        // At 50%: green
        assert_eq!(g.rgb_at(50), (0, 255, 0));
        // At 100%: blue
        assert_eq!(g.rgb_at(100), (0, 0, 255));
    }

    #[test]
    fn clamped_beyond_100() {
        let g = Gradient::two_color((0, 0, 0), (255, 255, 255));
        assert_eq!(g.rgb_at(200), (255, 255, 255));
    }

    #[test]
    fn flat_is_uniform() {
        let g = Gradient::flat((128, 64, 32));
        assert_eq!(g.rgb_at(0), (128, 64, 32));
        assert_eq!(g.rgb_at(50), (128, 64, 32));
        assert_eq!(g.rgb_at(100), (128, 64, 32));
    }

    #[test]
    fn tty_three_step_sections() {
        let g = Gradient::tty_three_step(Color::Green, Color::Yellow, Color::Red);
        assert_eq!(g.at(0), Color::Green);
        assert_eq!(g.at(33), Color::Green);
        assert_eq!(g.at(34), Color::Yellow);
        assert_eq!(g.at(66), Color::Yellow);
        assert_eq!(g.at(67), Color::Red);
        assert_eq!(g.at(100), Color::Red);
    }
}
