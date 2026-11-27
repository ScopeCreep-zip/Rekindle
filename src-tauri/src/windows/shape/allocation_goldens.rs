//! Native geometry controls from the independently reviewed SK0 art, plus
//! a pixel oracle for the logical projection across the installed packs.

use rekindle_types::dto::skin::SkinWindow;
use rekindle_types::dto::skin_library::SkinFrames;
use tauri::LogicalSize;

use super::{logical::LogicalShape, sizing::Sizing, skin, ShapeRect, Silhouette};

fn frames(id: &str) -> skin::SkinFrameMap {
    let (_, pack) = rekindle_skin::builtin::find(id).unwrap();
    let canonical = rekindle_skin::validate::run(pack.bytes).unwrap();
    let address = canonical.address();
    skin::decode(SkinFrames {
        id: id.to_owned(),
        tokens_href: rekindle_skin::serve::tokens_href(&address),
        windows: rekindle_skin::serve::frames(&canonical.bytes, &address).unwrap(),
    })
    .unwrap()
}

fn shape(
    frames: &skin::SkinFrameMap,
    frame: SkinWindow,
    canvas: (i32, i32),
    scale: i32,
) -> Silhouette {
    let logical = LogicalSize::new(f64::from(canvas.0), f64::from(canvas.1));
    skin::compute(
        frames,
        Some(frame),
        logical.to_physical(f64::from(scale)),
        f64::from(scale),
    )
    .unwrap()
}

#[test]
fn cosmic_geometry_controls_are_in_surface_coordinates() {
    for (id, canvas, expected) in [
        ("builtin:stealthx", (400, 650), (112, 0, 288, 650)),
        ("builtin:stealthx", (368, 650), (112, 0, 256, 650)),
        ("builtin:combat", (340, 651), (0, 0, 340, 650)),
        ("builtin:combat", (368, 650), (0, 0, 368, 649)),
        ("builtin:symbiosis", (368, 650), (0, 0, 368, 650)),
    ] {
        let source = shape(&frames(id), SkinWindow::Main, canvas, 1);
        let region = LogicalShape::project(&source, canvas, 1).unwrap();
        assert_eq!(region.bounds, expected, "{id} {canvas:?}");
    }
}

/// Raster oracle. Each physical rectangle touches whole logical pixels if
/// any of their physical pixels intersect it. No unrelated frame pixels
/// are filled to make its bounds rectangular.
fn raster(rects: &[ShapeRect], canvas: (i32, i32), scale: i32) -> Vec<bool> {
    let area = usize::try_from(canvas.0 * canvas.1).unwrap();
    let mut result = vec![false; area];
    for rect in rects {
        let right = rect.x + i32::try_from(rect.w).unwrap();
        let bottom = rect.y + i32::try_from(rect.h).unwrap();
        for y in rect.y.div_euclid(scale).max(0)..((bottom + scale - 1) / scale).min(canvas.1) {
            let start = y * canvas.0 + rect.x.div_euclid(scale).max(0);
            let end = y * canvas.0 + ((right + scale - 1) / scale).min(canvas.0);
            result[usize::try_from(start).unwrap()..usize::try_from(end).unwrap()].fill(true);
        }
    }
    result
}

#[test]
fn all_builtin_frames_keep_their_masks_at_minimum_preferred_and_larger_allocations() {
    for pack in rekindle_skin::builtin::PACKS {
        let id = format!("builtin:{}", pack.id);
        let frames = frames(&id);
        for (&frame, (layout, _)) in &frames {
            let policy = Sizing::for_frame(frame, Some(layout));
            let pixels = |(w, h)| LogicalSize::new(w, h).cast::<i32>();
            let min = pixels(policy.minimum);
            let preferred = pixels(policy.wanted);
            let larger = pixels((
                (policy.wanted.0 + 61.0).min(policy.maximum.0.unwrap_or(f64::MAX)),
                (policy.wanted.1 + 47.0).min(policy.maximum.1.unwrap_or(f64::MAX)),
            ));
            for size in [min, preferred, larger] {
                let canvas = (size.width, size.height);
                for scale in [1, 2] {
                    let source = shape(&frames, frame, canvas, scale);
                    let logical = LogicalShape::project(&source, canvas, scale).unwrap();
                    assert_eq!(
                        logical.margins.visible(canvas),
                        (logical.bounds.2, logical.bounds.3)
                    );
                    let regions: Vec<_> = logical
                        .rects
                        .iter()
                        .map(|&(x, y, w, h)| ShapeRect {
                            x,
                            y,
                            w: u32::try_from(w).unwrap(),
                            h: u32::try_from(h).unwrap(),
                        })
                        .collect();
                    assert!(
                        raster(&regions, canvas, 1) == raster(source.rects(), canvas, scale),
                        "{id} {frame:?} {canvas:?} at {scale}×"
                    );
                }
            }
        }
    }
}
