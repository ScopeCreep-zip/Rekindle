//! A node's spans as stylesheet declarations (plan SK2, design §5 "CSS
//! emission"; split from [`super::css`] for plan SK17-3): each axis placed
//! as [`NodeSpan::place`] places it, in whole pixels at every parent size.
//! A [`NodeSpan::Share`] is `permille`/1000 of the parent's length floored
//! (`round(down, …, 1px)`, CSS's floor), and a span chained after a
//! sibling ([`Anchor::After`]) that the Host could not resolve to a number
//! (one after a share: Combat's chat `Divider` on `ChatArea.BOTTOM`) starts
//! at that sibling's far edge as the sibling's own expression, as
//! [`rekindle_types::dto::skin_tree::SkinTree::boxes`] places it.

use rekindle_types::dto::skin_tree::{Anchor, NodeSpan, SkinNode};

/// `permille`/1000 of the CSS length `len`, floored as
/// [`rekindle_types::dto::skin_tree::share_of`] floors.
fn share(len: &str, permille: u16) -> String {
    format!("round(down, calc(({len}) * {permille} / 1000), 1px)")
}

/// The sibling `sibling` among `prev` (the nodes laid before this one in
/// the same parent) and the siblings laid before it.
fn sibling<'a>(prev: &'a [SkinNode], sibling: &str) -> Option<(&'a SkinNode, &'a [SkinNode])> {
    let j = prev
        .iter()
        .position(|n| n.id.eq_ignore_ascii_case(sibling))?;
    Some((&prev[j], &prev[..j]))
}

/// Where a span `size` (a CSS length) long anchored by `at` starts, from
/// the parent's start edge, as a CSS length; `x` picks the axis a chained
/// sibling is read on.
fn anchored(at: &Anchor, size: &str, prev: &[SkinNode], x: bool) -> String {
    match at {
        Anchor::Start(o) => format!("{o}px"),
        Anchor::End(o) => format!("(100% - {o}px - {size})"),
        Anchor::Centre { by, before, after } => {
            let (b, a) = (i32::from(*before), i32::from(*after));
            format!(
                "({}px + round(down, calc((100% + {}px - {size}) / 2), 1px))",
                i32::from(*by) - b,
                b + a
            )
        }
        Anchor::After { sibling: name, gap } => match sibling(prev, name) {
            Some((sib, before)) => {
                let (s, l) = start_len(if x { &sib.x } else { &sib.y }, before, x);
                format!("({s} + {l} + {gap}px)")
            }
            None => format!("{gap}px"),
        },
    }
}

/// `span`'s start (from the parent's start edge) and length, as CSS
/// lengths.
fn start_len(span: &NodeSpan, prev: &[SkinNode], x: bool) -> (String, String) {
    match span {
        NodeSpan::Fixed { at, size } => {
            let size = format!("{size}px");
            (anchored(at, &size, prev, x), size)
        }
        NodeSpan::Stretch { start, end } => (
            format!("{start}px"),
            format!("(100% - {}px)", i32::from(*start) + i32::from(*end)),
        ),
        NodeSpan::Share {
            at: at @ Anchor::After { .. },
            permille,
        } => {
            let start = anchored(at, "0px", prev, x);
            let len = share(&format!("100% - {start}"), *permille);
            (start, len)
        }
        NodeSpan::Share { at, permille } => {
            let len = share("100%", *permille);
            (anchored(at, &len, prev, x), len)
        }
    }
}

/// A share's length or a chained span's start depends on the parent's
/// length or a sibling's box: both as `calc()`s.
fn by_expression(s: &NodeSpan, prev: &[SkinNode], (start, len): (&str, &str)) -> String {
    let (st, l) = start_len(s, prev, start == "left");
    format!("{start}: calc({st}); {len}: calc({l});")
}

/// One axis's declarations: `(start, end, len)` are its property names,
/// `prev` the siblings laid before the node, `text` a text-sized node.
pub(super) fn axis(
    s: &NodeSpan,
    prev: &[SkinNode],
    (start, end, len): (&str, &str, &str),
    text: bool,
) -> String {
    match s {
        NodeSpan::Fixed {
            at: Anchor::Start(o) | Anchor::Centre { by: o, .. },
            size: 0,
        } if text => format!("{start}: {o}px; {len}: max-content;"),
        NodeSpan::Fixed {
            at: Anchor::End(o),
            size: 0,
        } if text => format!("{end}: {o}px; {len}: max-content;"),
        NodeSpan::Fixed {
            at: Anchor::After { sibling: name, gap },
            size,
        } if sibling(prev, name).is_none() => format!("{start}: {gap}px; {len}: {size}px;"),
        NodeSpan::Fixed {
            at: Anchor::Start(o),
            size,
        } => format!("{start}: {o}px; {len}: {size}px;"),
        NodeSpan::Fixed {
            at: Anchor::End(o),
            size,
        } => format!("{end}: {o}px; {len}: {size}px;"),
        // Lead decision 32: the floor `NodeSpan::place` takes, in whole
        // pixels at every parent size (`round(down, …, 1px)` is CSS's
        // floor, as the frame's centred tiles use, `shape/css.rs`).
        NodeSpan::Fixed {
            at: Anchor::Centre { by, before, after },
            size,
        } => {
            let (b, a) = (i32::from(*before), i32::from(*after));
            let k = b + a - i32::from(*size);
            let sign = if k < 0 { '-' } else { '+' };
            format!(
                "{start}: calc({}px + round(down, calc((100% {sign} {}px) / 2), 1px)); {len}: {size}px;",
                i32::from(*by) - b,
                k.abs()
            )
        }
        NodeSpan::Stretch { start: s, end: e } => format!("{start}: {s}px; {end}: {e}px;"),
        NodeSpan::Fixed {
            at: Anchor::After { .. },
            ..
        }
        | NodeSpan::Share { .. } => by_expression(s, prev, (start, len)),
    }
}

/// The length a served target box `area` px long around art `art` px
/// long takes on an axis laid at `s` (plan SK7): a fixed span's served
/// box, a share's share grown by the same margin (laid by the parent, never
/// frozen at the smallest window's px); `None` for a stretch, whose insets
/// already hold the box.
pub(super) fn target_len(
    s: &NodeSpan,
    prev: &[SkinNode],
    x: bool,
    (area, art): (i32, i32),
) -> Option<String> {
    match s {
        NodeSpan::Fixed { .. } => Some(format!("{area}px")),
        NodeSpan::Share { .. } => Some(format!(
            "calc({} + {}px)",
            start_len(s, prev, x).1,
            area - art
        )),
        NodeSpan::Stretch { .. } => None,
    }
}
