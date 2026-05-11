//! Colored area graph — half-block (▀▄) technique for per-pixel gradient.
//!
//! Each terminal cell becomes two independently-colored pixels (upper
//! half and lower half). This provides double vertical resolution
//! compared to full-block rendering, with per-pixel gradient coloring.

use std::collections::VecDeque;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use super::super::palette::gradient::Gradient;

/// Colored area graph using half-block characters.
pub struct ColoredAreaGraph<'a> {
    /// Data values 0.0-100.0 in chronological order.
    pub data: &'a VecDeque<f64>,
    /// Color gradient for height-based coloring.
    pub gradient: &'a Gradient,
}

impl Widget for &ColoredAreaGraph<'_> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let pixel_height = area.height as usize * 2;
        let data_len = self.data.len();
        let data_offset = data_len.saturating_sub(area.width as usize);

        for col in 0..area.width {
            let data_idx = data_offset + col as usize;
            let value = if data_idx < data_len {
                self.data[data_idx]
            } else {
                0.0
            };
            let fill_pixels = (value / 100.0 * pixel_height as f64).round() as usize;

            for row in 0..area.height {
                let pixel_row_lower = (area.height - 1 - row) as usize * 2;
                let pixel_row_upper = pixel_row_lower + 1;

                let lower_filled = fill_pixels > pixel_row_lower;
                let upper_filled = fill_pixels > pixel_row_upper;

                let cell = &mut buf[(area.x + col, area.y + row)];

                match (upper_filled, lower_filled) {
                    (false, false) => { /* leave empty */ }
                    (false, true) => {
                        let pct = (pixel_row_lower * 100 / pixel_height).min(100);
                        cell.set_char('▄').set_fg(self.gradient.at(pct));
                    }
                    (true, false) => {
                        let pct = (pixel_row_upper * 100 / pixel_height).min(100);
                        cell.set_char('▀').set_fg(self.gradient.at(pct));
                    }
                    (true, true) => {
                        let upper_pct = (pixel_row_upper * 100 / pixel_height).min(100);
                        let lower_pct = (pixel_row_lower * 100 / pixel_height).min(100);
                        cell.set_char('▀')
                            .set_fg(self.gradient.at(upper_pct))
                            .set_bg(self.gradient.at(lower_pct));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_data_renders_nothing() {
        let data = VecDeque::new();
        let gradient = Gradient::two_color((0, 255, 0), (255, 0, 0));
        let graph = ColoredAreaGraph { data: &data, gradient: &gradient };
        let area = Rect::new(0, 0, 10, 5);
        let mut buf = Buffer::empty(area);
        (&graph).render(area, &mut buf);
        // All cells should be empty (space)
        for y in 0..5 {
            for x in 0..10 {
                assert_eq!(buf[(x, y)].symbol(), " ");
            }
        }
    }

    #[test]
    fn full_value_fills_column() {
        let mut data = VecDeque::new();
        data.push_back(100.0);
        let gradient = Gradient::flat((255, 0, 0));
        let graph = ColoredAreaGraph { data: &data, gradient: &gradient };
        let area = Rect::new(0, 0, 1, 3);
        let mut buf = Buffer::empty(area);
        (&graph).render(area, &mut buf);
        // All cells in column 0 should be filled
        for y in 0..3 {
            assert_ne!(buf[(0, y)].symbol(), " ", "row {y} should be filled");
        }
    }
}
