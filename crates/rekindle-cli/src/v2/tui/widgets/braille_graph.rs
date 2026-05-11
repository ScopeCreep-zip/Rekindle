//! Braille graph — btop-style dual-value-per-cell encoding.
//!
//! Each terminal cell uses a 2-column braille character where the left
//! column represents the previous data point and the right column the
//! current data point. This doubles effective horizontal resolution.
//!
//! The encoding: each data point is quantized to 0-4 (5 levels within
//! a single row). Two values (prev, cur) combine into a single index:
//! prev * 5 + cur, selecting from a 25-element lookup table.

use std::collections::VecDeque;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;

use super::super::palette::gradient::Gradient;

/// 25 braille symbols for the "up" direction (bottom-to-top fill).
const BRAILLE_UP: [&str; 25] = [
    " ", "⢀", "⢠", "⢰", "⢸",
    "⡀", "⣀", "⣠", "⣰", "⣸",
    "⡄", "⣄", "⣤", "⣴", "⣼",
    "⡆", "⣆", "⣦", "⣶", "⣾",
    "⡇", "⣇", "⣧", "⣷", "⣿",
];

/// Ghost background character — faint pattern showing graph extent.
const GHOST_CHAR: &str = "⣀";

/// Braille graph widget with per-row gradient coloring and ghost background.
pub struct BrailleGraph<'a> {
    /// Data values 0.0-100.0 in chronological order.
    pub data: &'a VecDeque<f64>,
    /// Color gradient for value-based coloring.
    pub gradient: &'a Gradient,
    /// Ghost background color (faint graph extent indicator).
    pub ghost_fg: ratatui::style::Color,
    /// Whether to show the ghost background.
    pub show_ghost: bool,
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
impl Widget for &BrailleGraph<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let width = area.width as usize;
        let height = area.height as usize;
        // Each cell encodes 2 data points horizontally
        let data_points_needed = width * 2;

        // Pass 1: ghost background
        if self.show_ghost {
            let ghost_style = Style::new().fg(self.ghost_fg);
            for row in 0..area.height {
                for col in 0..area.width {
                    buf[(area.x + col, area.y + row)]
                        .set_symbol(GHOST_CHAR)
                        .set_style(ghost_style);
                }
            }
        }

        // Pass 2: actual data
        let data_len = self.data.len();
        let data_offset = data_len.saturating_sub(data_points_needed);

        for row in 0..height {
            // Row 0 is top of graph (highest values), row height-1 is bottom
            let row_bottom = (height - 1 - row) as f64 * 100.0 / height as f64;
            let row_top = (height - row) as f64 * 100.0 / height as f64;

            for col in 0..width {
                let data_idx_base = data_offset + col * 2;
                let prev_val = if data_idx_base < data_len {
                    self.data[data_idx_base]
                } else {
                    0.0
                };
                let cur_val = if data_idx_base + 1 < data_len {
                    self.data[data_idx_base + 1]
                } else {
                    0.0
                };

                let prev_dots = quantize(prev_val, row_bottom, row_top);
                let cur_dots = quantize(cur_val, row_bottom, row_top);

                // Skip empty cells (don't overwrite ghost)
                if prev_dots == 0 && cur_dots == 0 {
                    continue;
                }

                let symbol_idx = prev_dots * 5 + cur_dots;
                let symbol = BRAILLE_UP[symbol_idx];

                // Color based on the higher of the two values
                let color_pct = (prev_val.max(cur_val) as usize).min(100);
                let color = self.gradient.at(color_pct);

                buf[(area.x + col as u16, area.y + row as u16)]
                    .set_symbol(symbol)
                    .set_fg(color);
            }
        }
    }
}

/// Quantize a value (0-100) to a dot level (0-4) within a row's vertical range.
///
/// The MOD bias (0.1) prevents invisible low values — a value just above
/// the row's bottom boundary still produces at least 1 dot.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn quantize(value: f64, row_bottom: f64, row_top: f64) -> usize {
    const MOD: f64 = 0.1;
    if value >= row_top {
        return 4;
    }
    if value <= row_bottom {
        return 0;
    }
    let range = row_top - row_bottom;
    if range <= 0.0 {
        return 0;
    }
    let normalized = (value - row_bottom) / range;
    ((normalized * 4.0 + MOD).round() as usize).clamp(0, 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_boundaries() {
        assert_eq!(quantize(0.0, 0.0, 25.0), 0);
        assert_eq!(quantize(25.0, 0.0, 25.0), 4);
        assert_eq!(quantize(12.5, 0.0, 25.0), 2);
    }

    #[test]
    fn quantize_below_range() {
        assert_eq!(quantize(0.0, 50.0, 100.0), 0);
    }

    #[test]
    fn quantize_above_range() {
        assert_eq!(quantize(100.0, 0.0, 50.0), 4);
    }

    #[test]
    fn braille_symbol_table_size() {
        assert_eq!(BRAILLE_UP.len(), 25);
    }

    #[test]
    fn braille_endpoints() {
        // (0,0) = space, (4,4) = full block
        assert_eq!(BRAILLE_UP[0], " ");
        assert_eq!(BRAILLE_UP[24], "⣿");
    }
}
