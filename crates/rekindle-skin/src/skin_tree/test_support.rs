//! Shared fixtures for the skin-tree import tests: an empty frame and solid
//! art pieces sized by their names.

use rekindle_image::{Rgba, RgbaImage};
use rekindle_types::dto::skin::{Insets, WindowLayout};

/// No insets on any side.
pub(super) const Z: Insets = Insets {
    top: 0,
    right: 0,
    bottom: 0,
    left: 0,
};

/// An empty frame with a small declared minimum, the importer's start.
pub(super) fn frame() -> WindowLayout {
    WindowLayout {
        outsets: Z,
        content: Z,
        min_width: 30,
        min_height: 20,
        tiles: Vec::new(),
        title: None,
        tree: None,
    }
}

/// A solid image sized by its name's `-WxH` suffix.
pub(super) fn piece(path: &str) -> RgbaImage {
    let (w, h) = path
        .rsplit_once('-')
        .and_then(|(_, d)| d.split_once('x'))
        .unwrap();
    RgbaImage::from_pixel(
        w.parse().unwrap(),
        h.parse().unwrap(),
        Rgba([10, 20, 30, 255]),
    )
}
