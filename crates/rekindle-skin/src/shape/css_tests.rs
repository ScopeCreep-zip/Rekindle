//! The frame rules (plan SK0 S2): goldens for xfire-classic's and Total
//! Chaos's main windows, and every built-in's tiles drawn where
//! [`super::super::silhouette`] lays them, at their source size.

use rekindle_types::dto::skin::{SkinManifest, SkinWindow, WindowLayout};

use super::*;
use crate::shape::silhouette::placed;
use crate::{builtin, images, manifest, package};

const BASE: &str = "skin://localhost/pkg";

fn pack(id: &str) -> (SkinManifest, BTreeMap<String, Vec<u8>>) {
    let (_, b) = builtin::find(id).unwrap();
    let p = package::read(b.bytes).unwrap();
    let m = manifest::parse(&p).unwrap();
    let files = images::validate(&p, &m).unwrap();
    (m, files)
}

/// The tile rules of `window` in `css`, in slot order, each image's href
/// replaced by its package path so a golden reads as the layout.
fn slots(id: &str, window: SkinWindow) -> Vec<String> {
    let (m, files) = pack(id);
    let css = frame_css(&m, &files, BASE).unwrap();
    let root = format!(
        "  .app-frame[data-skin-window=\"{}\"] > .skin-frame > :nth-child(",
        window_attr(window)
    );
    let mut out = Vec::new();
    for (n, chunk) in css.split(&root).skip(1).enumerate() {
        let (slot, body) = chunk.split_once(") {\n    ").unwrap();
        assert_eq!(slot, (n + 1).to_string(), "slots are numbered in order");
        let mut body = body.split_once("\n  }").unwrap().0.to_owned();
        for (path, bytes) in &files {
            let h = format!("{BASE}/{}.", blake3::hash(bytes).to_hex());
            if let Some(at) = body.find(&h) {
                let end = at + body[at..].find('"').unwrap();
                body.replace_range(at..end, path.trim_start_matches("assets/frame/main/"));
            }
        }
        out.push(body);
    }
    out
}

#[test]
fn xfire_classic_main_golden() {
    let want = [
        "display: block; left: 0px; width: 70px; top: 0px; height: 47px; background: url(\"top-left.png\") 0 0 / 70px 47px no-repeat;",
        "display: block; left: 70px; right: 70px; top: 0px; height: 46px; background: url(\"top.png\") 0 0 / 100px 46px repeat-x;",
        "display: block; right: 0px; width: 70px; top: 0px; height: 47px; background: url(\"top-right.png\") 0 0 / 70px 47px no-repeat;",
        "display: block; left: 0px; width: 7px; top: 47px; bottom: 62px; background: url(\"left.png\") 0 0 / 7px 236px repeat-y;",
        "display: block; right: 0px; width: 6px; top: 47px; bottom: 62px; background: url(\"right.png\") 0 0 / 6px 241px repeat-y;",
        "display: block; left: 0px; width: 117px; bottom: 0px; height: 62px; background: url(\"bottom-left.png\") 0 0 / 117px 62px no-repeat;",
        "display: block; left: 117px; right: 115px; bottom: 0px; height: 63px; background: url(\"bottom.png\") 0 0 / 100px 63px repeat-x;",
        "display: block; right: 0px; width: 115px; bottom: 0px; height: 62px; background: url(\"bottom-right.png\") 0 0 / 115px 62px no-repeat;",
    ];
    assert_eq!(slots("builtin:xfire-classic", SkinWindow::Main), want);
}

#[test]
fn total_chaos_main_golden() {
    let got = slots("builtin:total-chaos", SkinWindow::Main);
    assert_eq!(got.len(), 15);
    // The row stack: left/right pieces at the edges, middles repeating
    // between them at source size.
    assert_eq!(got[0], "display: block; left: 0px; width: 75px; top: 0px; height: 52px; background: url(\"row0-left.png\") 0 0 / 75px 52px no-repeat;");
    assert_eq!(got[1], "display: block; left: 75px; right: 75px; top: 0px; height: 52px; background: url(\"row0-middle.png\") 0 0 / 8px 52px repeat-x;");
    // The T.C plaque: centred on the window (an overhang), floored to a
    // whole pixel like the silhouette, painted over row 0's middle.
    assert_eq!(got[3], "display: block; left: calc(0px + round(down, calc((100% - 206px) / 2), 1px)); width: 206px; top: 0px; height: 52px; background: url(\"row0-centre.png\") 0 0 / 206px 52px no-repeat;");
    assert_eq!(got[4], "display: block; left: 0px; width: 59px; top: 52px; height: 60px; background: url(\"row1-left.png\") 0 0 / 59px 60px no-repeat;");
    assert_eq!(got[8], "display: block; left: 83px; right: 83px; top: 112px; height: 69px; background: url(\"row2-middle.png\") 0 0 / 30px 69px repeat-x;");
    assert_eq!(got[11], "display: block; right: 0px; width: 50px; top: 181px; bottom: 66px; background: url(\"row3-right.png\") 0 0 / 50px 37px repeat-y;");
    assert_eq!(got[14], "display: block; right: 0px; width: 161px; bottom: 0px; height: 66px; background: url(\"row4-right.png\") 0 0 / 161px 66px no-repeat;");
}

#[test]
fn the_frame_box_holds_the_content_in_its_insets_and_drops_the_border() {
    let (m, files) = pack("builtin:symbiosis");
    let css = frame_css(&m, &files, BASE).unwrap();
    assert!(css.starts_with("@layer shape {\n"));
    // The Infoview strip's 13 px outset on the right, then the 13 px insets.
    assert!(css.contains(
        "  .app-frame[data-skin-window=\"main\"] {\n    border: 0;\n    border-radius: 0;\n    border-image: none;\n    position: relative;\n    isolation: isolate;\n    padding: 13px 26px 13px 13px;\n    background: var(--color-xfire-bg-dark) content-box;\n"
    ));
    // The strip is a decoration: drawn after the eight frame pieces, at the
    // window's right edge (an overhang ignores the outsets).
    let strip = "  .app-frame[data-skin-window=\"main\"] > .skin-frame > :nth-child(9) {\n    display: block; right: 0px; width: 13px; top: calc(0px + round(down, calc((100% - 115px) / 2), 1px)); height: 115px;";
    assert!(css.contains(strip), "{css}");
}

#[test]
fn xfire_classic_pads_the_content_to_its_side_edges() {
    // S1f: the content insets are the side edge pieces (7 and 6 px), the
    // titlebar band (47 px) and the console row (63 px), not the corners.
    let (m, files) = pack("builtin:xfire-classic");
    let css = frame_css(&m, &files, BASE).unwrap();
    assert!(
        css.contains(
            "  .app-frame[data-skin-window=\"main\"] {\n    border: 0;\n    border-radius: 0;\n    border-image: none;\n    position: relative;\n    isolation: isolate;\n    padding: 47px 6px 63px 7px;\n"
        ),
        "{css}"
    );
}

/// The px value of `prop` in `decls` at an axis of `len` px.
fn eval(decls: &str, prop: &str, len: i64) -> Option<i64> {
    let at = decls
        .find(&format!(" {prop}: "))
        .or_else(|| decls.starts_with(&format!("{prop}: ")).then_some(0))?;
    let v = decls[at..].split_once(": ")?.1.split_once(';')?.0;
    if let Some(px) = v.strip_suffix("px").and_then(|n| n.parse().ok()) {
        return Some(px);
    }
    // calc(Npx + round(down, calc((100% - Mpx) / 2), 1px))
    let n: i64 = v.strip_prefix("calc(")?.split_once("px")?.0.parse().ok()?;
    let m: i64 = v
        .split_once("100% - ")?
        .1
        .split_once("px")?
        .0
        .parse()
        .ok()?;
    Some(n + (len - m).div_euclid(2))
}

/// The element box of a tile along one axis: (start, end).
fn span(decls: &str, (start, end, size): (&str, &str, &str), len: i64) -> (i64, i64) {
    match (
        eval(decls, start, len),
        eval(decls, end, len),
        eval(decls, size, len),
    ) {
        (Some(s), None, Some(w)) => (s, s + w),
        (None, Some(e), Some(w)) => (len - e - w, len - e),
        (Some(s), Some(e), None) => (s, len - e),
        other => panic!("unexpected axis {other:?} in {decls}"),
    }
}

fn every_tile_drawn_where_laid(layout: &WindowLayout, rules: &[String], w: i64, h: i64) {
    let order = paint_order(layout);
    let copies = placed(layout, u32::try_from(w).unwrap(), u32::try_from(h).unwrap());
    for (slot, &i) in order.iter().enumerate() {
        let t = &layout.tiles[i];
        let d = &rules[slot];
        let mine: Vec<_> = copies
            .iter()
            .filter(|(k, _)| *k == i)
            .map(|(_, a)| *a)
            .collect();
        let (x0, x1) = span(d, ("left", "right", "width"), w);
        let (y0, y1) = span(d, ("top", "bottom", "height"), h);
        if let (Some(min_x), Some(min_y)) = (
            mine.iter().map(|a| a.0).min(),
            mine.iter().map(|a| a.1).min(),
        ) {
            let max_x = mine.iter().map(|a| a.0 + a.2).max().unwrap();
            let max_y = mine.iter().map(|a| a.1 + a.3).max().unwrap();
            assert_eq!(
                (x0, x1, y0, y1),
                (min_x, max_x, min_y, max_y),
                "tile {i}: {d}"
            );
        }
        // Drawn at its source size, from the box's top left: never stretched.
        let size = format!(") 0 0 / {}px {}px ", t.width, t.height);
        assert!(d.contains(&size), "tile {i} not at its source size: {d}");
    }
}

#[test]
fn every_builtin_draws_each_tile_where_the_silhouette_lays_it() {
    let mut windows = 0;
    for b in builtin::PACKS {
        let id = format!("{}{}", builtin::PREFIX, b.id);
        let (m, _) = pack(&id);
        for (&window, layout) in &m.windows {
            let rules = slots(&id, window);
            assert_eq!(rules.len(), layout.tiles.len(), "{id} {window:?}");
            let (min_w, min_h) = (i64::from(layout.min_width), i64::from(layout.min_height));
            let o = layout.outsets;
            let ow = i64::from(o.left) + i64::from(o.right);
            let oh = i64::from(o.top) + i64::from(o.bottom);
            for (w, h) in [
                (min_w + ow, min_h + oh),
                (min_w + ow + 333, min_h + oh + 517),
            ] {
                every_tile_drawn_where_laid(layout, &rules, w, h);
            }
            windows += 1;
        }
    }
    assert!(windows >= 8, "the built-ins lay {windows} windows");
}

#[test]
fn the_window_renders_as_many_slots_as_a_layout_may_have() {
    let tsx = include_str!("../../../../src/components/common/SkinFrame.tsx");
    assert!(
        tsx.contains(&format!("const FRAME_TILES = {MAX_FRAME_TILES};")),
        "SkinFrame.tsx FRAME_TILES must equal MAX_FRAME_TILES"
    );
}

#[test]
fn a_shaped_window_without_a_frame_keeps_square_corners() {
    // The shaper cuts an unlaid buddy list or chat window to its whole
    // rectangle (`serve::frames` lays nothing for it), so the page's
    // `.app-frame` radius must not round the drawing inside that cut.
    let css = include_str!("../../../../src/styles/theme/22-skin-frame.css");
    let rule = ".app-frame[data-skin-window] {\n    border-radius: 0;\n  }";
    assert!(
        css.contains(rule),
        "22-skin-frame.css must square a shaped window: {css}"
    );
}

#[test]
fn the_header_region_places_the_profile_header_over_the_banner() {
    // Total Chaos's `AdArea` (236×60, centred at 52 px): the profile header
    // leaves the content's flow and sits there (plan SK0-L1).
    let rule = |id: &str| {
        let (m, files) = pack(id);
        let css = frame_css(&m, &files, BASE).unwrap();
        css.split_once("  .app-frame[data-skin-window=\"main\"] > .identity-bar {\n")
            .map(|(_, r)| r.split_once("\n  }").unwrap().0.to_owned())
    };
    assert_eq!(
        rule("builtin:total-chaos").as_deref(),
        Some("    position: absolute;\n    left: calc(0px + round(down, calc((100% - 236px) / 2), 1px)); width: 236px; top: 52px; height: 60px;\n    box-sizing: border-box;\n    margin: 0;\n    overflow: hidden;\n    z-index: 1;")
    );
    assert_eq!(rule("builtin:xfire-classic"), None);
}
