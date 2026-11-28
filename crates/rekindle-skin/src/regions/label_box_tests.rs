use super::*;
use rekindle_image::Rgba;

/// A 16 × 24 tile, black but for light rows above and below a group
/// header's text line (`.buddy-group-header`: 4 px top padding, an
/// 11 px line, rows 4-20 of a 24 px box).
fn banded() -> RgbaImage {
    RgbaImage::from_fn(16, 24, |_, y| {
        if (4..=20).contains(&y) {
            Rgba([0, 0, 0, 255])
        } else {
            Rgba([240, 240, 240, 255])
        }
    })
}

#[test]
fn art_light_only_outside_the_label_rows_carries_the_label() {
    let image = banded();
    let text = [204, 204, 204, 255];
    let extents = (Extent::Window, Extent::Range { min: 24, max: 24 });
    let under = label_worst(
        "buddylist.group-header",
        extents,
        0,
        &image,
        RegionFit::Tile,
        None,
        text,
    )
    .expect("a fit");
    assert!(under >= 4.5, "{under}");
    // The light rows exist: measured over every pixel the art fails.
    let all: Vec<u32> = (0..24).collect();
    let cols: Vec<u32> = (0..16).collect();
    let every = super::super::worst_contrast(&image, &cols, &all, None, [0; 4], text);
    assert!(every < 4.5, "{every}");
}

#[test]
fn a_centred_line_covers_the_middle_rows_only() {
    let g = label(8, 0, 0, Line::Centred(LINE_11));
    // 24 px box: the 16.5 px line spans 3.75-20.25 px.
    assert_eq!(line_rows(g, 24), (4, 20));
    assert_eq!(line_rows(label(0, 0, 0, Line::Fill), 24), (0, 24));
}
