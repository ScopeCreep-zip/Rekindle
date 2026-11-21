//! One stylesheet rule per tree node (plan SK2, design §5 "CSS emission"):
//! `[data-skin-node="<id>"]` placed absolutely in its parent's node by its
//! resolved spans, at its `Z`, with its art at its own size and its hover,
//! down and disabled art; an interactive node smaller than 24×24 gets a
//! 24×24 hit area (`::before`, centred). Positions never come from the
//! webview; the shell (SK2-S2) only nests `div[data-skin-node]`s.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use rekindle_types::dto::skin::RegionState;
use rekindle_types::dto::skin_tree::{Anchor, SkinNode, SkinTree, Span};

use super::validate::HIT;
use crate::regions::css::href;

fn axis(s: Span, (start, end, len): (&str, &str, &str)) -> String {
    match s {
        Span::Fixed {
            at: Anchor::Start(o),
            size,
        } => format!("{start}: {o}px; {len}: {size}px;"),
        Span::Fixed {
            at: Anchor::End(o),
            size,
        } => format!("{end}: {o}px; {len}: {size}px;"),
        Span::Fixed {
            at: Anchor::Centre(o),
            size,
        } => {
            format!(
                "{start}: calc(50% - {}px); {len}: {size}px;",
                i32::from(size) / 2 - i32::from(o)
            )
        }
        Span::Stretch { start: s, end: e } => format!("{start}: {s}px; {end}: {e}px;"),
    }
}

fn size(s: Span) -> Option<u16> {
    match s {
        Span::Fixed { size, .. } => Some(size),
        Span::Stretch { .. } => None,
    }
}

fn node(
    out: &mut String,
    root: &str,
    n: &SkinNode,
    images: &BTreeMap<String, Vec<u8>>,
    base: &str,
) -> Option<()> {
    let sel = format!("{root} [data-skin-node=\"{}\"]", n.id);
    let mut decls = format!(
        "position: absolute; {} {} z-index: {};",
        axis(n.x, ("left", "right", "width")),
        axis(n.y, ("top", "bottom", "height")),
        n.z
    );
    if let Some(art) = &n.art {
        let h = href(base, &art.image, images)?;
        let _ = write!(decls, " background: url(\"{h}\") 0 0 no-repeat;");
    }
    let _ = writeln!(out, "  {sel} {{\n    {decls}\n  }}");
    for (state, path) in n.art.iter().flat_map(|a| a.states.iter()) {
        let pseudo = match state {
            RegionState::Hover => ":hover",
            RegionState::Down => ":active",
            RegionState::Disabled => "[aria-disabled=\"true\"]",
            _ => continue,
        };
        let h = href(base, path, images)?;
        let _ = writeln!(
            out,
            "  {sel}{pseudo} {{\n    background-image: url(\"{h}\");\n  }}"
        );
    }
    let small = |s: Span| size(s).is_some_and(|v| i32::from(v) < HIT);
    if n.role.interactive() && (small(n.x) || small(n.y)) {
        let _ = writeln!(
            out,
            "  {sel}::before {{\n    content: \"\";\n    position: absolute;\n    left: 50%;\n    top: 50%;\n    width: max(100%, {HIT}px);\n    height: max(100%, {HIT}px);\n    translate: -50% -50%;\n  }}"
        );
    }
    for c in &n.children {
        node(out, root, c, images, base)?;
    }
    Some(())
}

/// The rules of `tree` under `root` (the window's `.app-frame` selector);
/// `None` for an art image `images` lacks (never for a validated package).
#[must_use]
pub fn tree_css(
    root: &str,
    tree: &SkinTree,
    images: &BTreeMap<String, Vec<u8>>,
    base: &str,
) -> Option<String> {
    let mut out = String::new();
    for n in &tree.root {
        node(&mut out, root, n, images, base)?;
    }
    Some(out)
}
