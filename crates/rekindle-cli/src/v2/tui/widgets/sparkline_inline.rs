//! Inline sparkline — compact single-row graph using ▁▂▃▄▅▆▇█.
//!
//! Used for: inline CPU/activity bars in process lists, member lists,
//! and any context where a 5-10 character wide mini-graph is needed.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use super::super::palette::gradient::Gradient;

/// The 9 bar characters representing 0/8 through 8/8 fill.
const BAR_CHARS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// Inline sparkline widget — renders a compact single-row graph.
pub struct InlineSparkline<'a> {
    /// Data values (any range — normalized to max).
    pub data: &'a [f64],
    /// Color gradient for value-based coloring.
    pub gradient: &'a Gradient,
    /// Maximum value for normalization. 0.0 = auto-detect from data.
    pub max_value: f64,
}

impl Widget for &InlineSparkline<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.data.is_empty() {
            return;
        }

        let max = if self.max_value > 0.0 {
            self.max_value
        } else {
            self.data.iter().copied().fold(0.0_f64, f64::max).max(1.0)
        };

        let width = area.width as usize;
        let data_offset = self.data.len().saturating_sub(width);

        for i in 0..width.min(self.data.len()) {
            let data_idx = data_offset + i;
            if data_idx >= self.data.len() {
                break;
            }

            let value = self.data[data_idx];
            let normalized = (value / max).clamp(0.0, 1.0);

            // Quantize to 0-8 levels
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let level = (normalized * 8.0).round() as usize;
            let symbol = BAR_CHARS[level.min(8)];

            // Color from gradient based on percentage
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let pct = (normalized * 100.0).round() as usize;
            let color = self.gradient.at(pct.min(100));

            #[allow(clippy::cast_possible_truncation)]
            buf[(area.x + i as u16, area.y)]
                .set_symbol(symbol)
                .set_fg(color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_chars_count() {
        assert_eq!(BAR_CHARS.len(), 9);
    }

    #[test]
    fn empty_data_renders_nothing() {
        let data: &[f64] = &[];
        let gradient = Gradient::flat((0, 255, 0));
        let sparkline = InlineSparkline { data, gradient: &gradient, max_value: 100.0 };
        let area = Rect::new(0, 0, 5, 1);
        let mut buf = Buffer::empty(area);
        (&sparkline).render(area, &mut buf);
        for x in 0..5 {
            assert_eq!(buf[(x, 0)].symbol(), " ");
        }
    }

    #[test]
    fn max_value_renders_full_block() {
        let data = [100.0];
        let gradient = Gradient::flat((255, 0, 0));
        let sparkline = InlineSparkline { data: &data, gradient: &gradient, max_value: 100.0 };
        let area = Rect::new(0, 0, 1, 1);
        let mut buf = Buffer::empty(area);
        (&sparkline).render(area, &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), "█");
    }

    #[test]
    fn half_value_renders_half_block() {
        let data = [50.0];
        let gradient = Gradient::flat((0, 255, 0));
        let sparkline = InlineSparkline { data: &data, gradient: &gradient, max_value: 100.0 };
        let area = Rect::new(0, 0, 1, 1);
        let mut buf = Buffer::empty(area);
        (&sparkline).render(area, &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), "▄");
    }
}
