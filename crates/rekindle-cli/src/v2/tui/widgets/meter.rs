//! Gradient meter — horizontal bar with per-cell gradient coloring.
//!
//! Uses ■ (U+25A0 BLACK SQUARE) for filled cells, dim ■ for unfilled.
//! Each filled cell gets a color from the gradient based on its position.
//! Optionally inverts the gradient direction (right-to-left).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;

use super::super::palette::gradient::Gradient;

/// Gradient meter widget.
pub struct GradientMeter<'a> {
    /// Value 0-100.
    pub value: u8,
    /// Color gradient for filled cells.
    pub gradient: &'a Gradient,
    /// Background color for unfilled cells.
    pub bg_color: ratatui::style::Color,
    /// Whether to invert the gradient direction.
    pub invert: bool,
}

impl Widget for &GradientMeter<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let value = u16::from(self.value.min(100));
        #[allow(clippy::cast_possible_truncation)]
        let fill_width = (u32::from(area.width) * u32::from(value) / 100) as u16;

        for i in 0..area.width {
            let cell = &mut buf[(area.x + i, area.y)];
            if i < fill_width {
                let pct = if self.invert {
                    100 - (i as usize * 100 / area.width as usize)
                } else {
                    i as usize * 100 / area.width as usize
                };
                cell.set_symbol("■").set_fg(self.gradient.at(pct));
            } else {
                cell.set_symbol("■").set_style(Style::new().fg(self.bg_color));
            }
        }
    }
}

/// Cached gradient meter — pre-computes all 101 possible renderings.
///
/// For widgets that render meters frequently at the same width (e.g.,
/// process list with 200 rows each showing a CPU meter), caching
/// eliminates redundant gradient lookups.
pub struct CachedMeter {
    width: u16,
    gradient: Gradient,
    bg_color: ratatui::style::Color,
    cache: Vec<Option<Vec<(char, ratatui::style::Color)>>>,
}

impl CachedMeter {
    /// Create a new cached meter with the given width and gradient.
    pub fn new(width: u16, gradient: Gradient, bg_color: ratatui::style::Color) -> Self {
        Self {
            width,
            gradient,
            bg_color,
            cache: vec![None; 101],
        }
    }

    /// Get the pre-computed cell data for a given percentage.
    pub fn get(&mut self, value: u8) -> &[(char, ratatui::style::Color)] {
        let idx = value.min(100) as usize;
        let width = self.width;
        let gradient = &self.gradient;
        let bg = self.bg_color;
        self.cache[idx].get_or_insert_with(|| {
            #[allow(clippy::cast_possible_truncation)]
            let fill_width = (u32::from(width) * u32::from(value) / 100) as u16;
            (0..width)
                .map(|i| {
                    if i < fill_width {
                        let pct = i as usize * 100 / width as usize;
                        ('■', gradient.at(pct))
                    } else {
                        ('■', bg)
                    }
                })
                .collect()
        })
    }

    /// Render the cached meter at the given position.
    pub fn render_at(&mut self, value: u8, x: u16, y: u16, buf: &mut Buffer) {
        let cells = self.get(value);
        for (i, &(ch, color)) in cells.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            let col = i as u16;
            buf[(x + col, y)]
                .set_char(ch)
                .set_fg(color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn meter_zero_all_bg() {
        let gradient = Gradient::flat((0, 255, 0));
        let meter = GradientMeter {
            value: 0,
            gradient: &gradient,
            bg_color: Color::DarkGray,
            invert: false,
        };
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        (&meter).render(area, &mut buf);
        for x in 0..10 {
            assert_eq!(buf[(x, 0)].symbol(), "■");
            assert_eq!(buf[(x, 0)].fg, Color::DarkGray);
        }
    }

    #[test]
    fn meter_100_all_gradient() {
        let gradient = Gradient::flat((255, 0, 0));
        let meter = GradientMeter {
            value: 100,
            gradient: &gradient,
            bg_color: Color::DarkGray,
            invert: false,
        };
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        (&meter).render(area, &mut buf);
        for x in 0..10 {
            assert_eq!(buf[(x, 0)].symbol(), "■");
            assert_ne!(buf[(x, 0)].fg, Color::DarkGray);
        }
    }

    #[test]
    fn meter_50_half_filled() {
        let gradient = Gradient::flat((0, 255, 0));
        let meter = GradientMeter {
            value: 50,
            gradient: &gradient,
            bg_color: Color::DarkGray,
            invert: false,
        };
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        (&meter).render(area, &mut buf);
        // First 5 cells should be gradient-colored, last 5 should be bg
        assert_ne!(buf[(0, 0)].fg, Color::DarkGray);
        assert_eq!(buf[(5, 0)].fg, Color::DarkGray);
    }

    #[test]
    fn cached_meter_consistency() {
        let gradient = Gradient::two_color((0, 0, 0), (255, 255, 255));
        let mut cached = CachedMeter::new(10, gradient, Color::DarkGray);
        let first = cached.get(50).to_vec();
        let second = cached.get(50).to_vec();
        assert_eq!(first, second);
    }
}
