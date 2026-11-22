//! Tree-node art in the outline (plan SK2-S1r).

use rekindle_image::{encode_png, DynamicImage, Rgba, RgbaImage};
use rekindle_types::dto::skin::{Insets, SkinWindow};
use rekindle_types::dto::skin_tree::{
    Anchor, NodeArt, NodeRole, NodeSpan, SkinNode, SkinTree, ViewKey,
};

use super::*;

const ON: Rgba<u8> = Rgba([10, 20, 30, 255]);
const OFF: Rgba<u8> = Rgba([0, 0, 0, 0]);

fn nine() -> [RgbaImage; 9] {
    let solid = |w, h| RgbaImage::from_pixel(w, h, ON);
    [
        solid(4, 4),
        solid(2, 4),
        solid(4, 4),
        solid(4, 2),
        RgbaImage::from_pixel(1, 1, OFF),
        solid(4, 2),
        solid(4, 4),
        solid(2, 4),
        solid(4, 4),
    ]
}

/// A node at (`x`, `y`) of the tree's window, `w × h`, drawing `art`.
fn node(id: &str, role: NodeRole, (x, y, w, h): (i16, i16, u16, u16)) -> SkinNode {
    SkinNode {
        id: id.to_owned(),
        role,
        x: NodeSpan::Fixed {
            at: Anchor::Start(x),
            size: w,
        },
        y: NodeSpan::Fixed {
            at: Anchor::Start(y),
            size: h,
        },
        z: 0,
        art: Some(NodeArt::middle(format!("assets/tree/{id}.png"), w, h)),
        text: None,
        shows_with: None,
        control_group: None,
        triggers: Vec::new(),
        children: Vec::new(),
    }
}

fn png(img: &RgbaImage) -> Vec<u8> {
    encode_png(&DynamicImage::ImageRgba8(img.clone())).unwrap()
}

#[test]
fn shown_node_art_joins_the_outline_where_it_is_opaque() {
    // A 20 px frame in a 22 px window: the frame and the content never
    // reach x 0..2, so only node art can.
    let (mut layout, mut files) = from_nine(SkinWindow::Chat, &nine()).unwrap();
    layout.outsets.left = 2;
    // A band whose (1, 0) is transparent; a 2 × 4 image in a 2 × 2 box
    // (drawn clipped to its box); a hidden view's art; a button bar's
    // second page's art.
    let band = RgbaImage::from_fn(2, 4, |x, y| if (x, y) == (1, 0) { OFF } else { ON });
    let tall = RgbaImage::from_pixel(2, 4, ON);
    let mut servers = node("servers", NodeRole::View(ViewKey::Servers), (0, 10, 2, 2));
    servers.art = Some(NodeArt::middle("assets/tree/tall.png".to_owned(), 2, 4));
    let mut bar = node("bar", NodeRole::ButtonBar, (0, 0, 22, 20));
    bar.art = None;
    let mut first = node("first", NodeRole::Group, (0, 0, 22, 20));
    first.art = None;
    let second = node("second", NodeRole::Decor, (0, 13, 2, 2));
    bar.children = vec![first, second];
    let clipped = node("clipped", NodeRole::Decor, (0, 16, 2, 2));
    layout.tree = Some(SkinTree {
        outsets: Insets {
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
        },
        root: vec![
            node("band", NodeRole::Decor, (0, 5, 2, 4)),
            servers,
            bar,
            clipped,
        ],
    });
    for (path, img) in [
        ("band", &band),
        ("tall", &tall),
        ("second", &tall),
        ("clipped", &tall),
    ] {
        files.insert(format!("assets/tree/{path}.png"), png(img));
    }
    let masks = of_layout("windows.chat", &layout, &files).unwrap();
    assert_eq!(masks.len(), layout.tiles.len() + 2, "band and clipped only");
    let rects = silhouette(&layout, &masks, 22, 20, SCALE_ONE).unwrap();
    let inside = |x: u32, y: u32| {
        rects
            .iter()
            .any(|r| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height)
    };
    let column = |y: u32| inside(0, y) && inside(1, y);
    assert!(
        inside(0, 5) && !inside(1, 5),
        "the band's transparent pixel"
    );
    assert!((6..9).all(column), "the band's art");
    assert!(!inside(0, 4) && !inside(0, 9));
    assert!(!(10..16).any(|y| inside(0, y)), "hidden view and page");
    assert!((16..18).all(column) && !inside(0, 18), "clipped to its box");
    // One mask short is refused.
    assert!(matches!(
        silhouette(&layout, &masks[..masks.len() - 1], 22, 20, SCALE_ONE),
        Err(ShapeError::Mask { .. })
    ));
}
