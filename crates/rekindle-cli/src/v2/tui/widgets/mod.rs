//! Custom visual widgets — braille graphs, colored area graphs, sparklines, gradient meters.
//!
//! These are the btop-level visual primitives built from scratch.
//! Each widget implements ratatui's Widget trait and uses the gradient
//! system for per-cell coloring.

pub mod braille_graph;
pub mod colored_area;
pub mod meter;
pub mod sparkline_inline;
