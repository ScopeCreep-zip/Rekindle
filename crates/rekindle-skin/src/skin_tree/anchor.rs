//! A tile's box on one axis from its symbolic anchors (plan SK2, design
//! §5 "Import"): a number, a sibling's edge ± n, a sum with a sibling's
//! size, `Just*`, `Resize*` with `Indent*`/`Outdent*`, `MaxWidth`/`MaxHeight`
//! caps, and the names a child sees in its own frame.

use std::collections::BTreeMap;

use rekindle_types::dto::skin_tree::{Anchor, NodeSpan};

use crate::import::tree::Node;
use crate::import::xfire_xml::title::reference;
use crate::import::ImportError;

use super::import::bad;

/// One axis's attribute names.
pub(super) struct Axis {
    pos: &'static str,
    just: &'static str,
    resize: &'static str,
    indent: [&'static str; 2],
    outdent: [&'static str; 2],
    far: &'static str,
    near: &'static str,
    size: &'static str,
    end_just: &'static str,
}

pub(super) const X: Axis = Axis {
    pos: "X",
    just: "JustX",
    resize: "ResizeX",
    indent: ["IndentLeft", "IndentRight"],
    outdent: ["OutdentLeft", "OutdentRight"],
    far: "RIGHT",
    near: "LEFT",
    size: "WIDTH",
    end_just: "RIGHT",
};

pub(super) const Y: Axis = Axis {
    pos: "Y",
    just: "JustY",
    resize: "ResizeY",
    indent: ["IndentTop", "IndentBottom"],
    outdent: ["OutdentTop", "OutdentBottom"],
    far: "BOTTOM",
    near: "TOP",
    size: "HEIGHT",
    end_just: "BOTTOM",
};

/// An edge's position: from the parent's start edge, from its end edge, or
/// from its centre (an edge of a centred sibling: Combat's `JoinPlayTab`
/// `X="CenterConsole.RIGHT"`).
enum At {
    Start(i32),
    End(i32),
    Centre(i32),
}

pub(super) type Sibs = BTreeMap<String, (NodeSpan, NodeSpan)>;

/// Each sibling's art size, known before it is placed: XGlow's `Middle`
/// ends `Bottom.height` above the bottom while `Bottom` sits on
/// `Middle.BOTTOM`, a cycle only in position, never in size.
pub(super) type Sizes = BTreeMap<String, (u16, u16)>;

fn span_of(sibs: &Sibs, name: &str, axis: &Axis) -> Option<NodeSpan> {
    sibs.get(&name.to_ascii_lowercase())
        .map(|s| if axis.pos == "X" { s.0 } else { s.1 })
}

/// A sum of signed terms, each a number or a sibling's `width`/`height`.
fn expr(v: Option<&str>, (sibs, sizes): (&Sibs, &Sizes), axis: &Axis) -> Result<i32, ImportError> {
    let Some(v) = v else { return Ok(0) };
    let mut total = 0i32;
    let mut rest = v.trim();
    let mut sign = 1;
    while !rest.is_empty() {
        let cut = rest[1..].find(['+', '-']).map_or(rest.len(), |i| i + 1);
        let (term, tail) = rest.split_at(cut);
        let term = term.trim();
        let (s, term) = match term.strip_prefix('-') {
            Some(t) => (-sign, t.trim()),
            None => (sign, term.trim_start_matches('+').trim()),
        };
        let value = if let Ok(num) = term.parse::<i32>() {
            num
        } else {
            let (name, field) = term
                .split_once('.')
                .ok_or_else(|| bad(format!("{v}: not a number")))?;
            if !field.eq_ignore_ascii_case(axis.size) {
                return Err(bad(format!("{v}: {field} is not this axis's size")));
            }
            let art = sizes
                .get(&name.to_ascii_lowercase())
                .map(|wh| if axis.pos == "X" { wh.0 } else { wh.1 })
                .filter(|s| *s > 0);
            match (span_of(sibs, name, axis), art) {
                (Some(NodeSpan::Fixed { size, .. }), _) | (None, Some(size)) => i32::from(size),
                _ => return Err(bad(format!("{v}: {name} has no fixed size"))),
            }
        };
        total += s * value;
        sign = 1;
        rest = tail.trim();
    }
    Ok(total)
}

/// `v` as a sibling's edge ± n, when it is one.
fn edge(value: &str, sibs: &Sibs, axis: &Axis) -> Result<Option<At>, ImportError> {
    let Some((name, side, by)) = reference(value) else {
        return Ok(None);
    };
    if side.eq_ignore_ascii_case(axis.size) {
        return Ok(None);
    }
    let far = side.eq_ignore_ascii_case(axis.far);
    if !far && !side.eq_ignore_ascii_case(axis.near) {
        return Err(bad(format!("{value}: {side} is not an edge of this axis")));
    }
    let sib = span_of(sibs, name, axis)
        .ok_or_else(|| bad(format!("{value}: no sibling {name} before it")))?;
    Ok(Some(match (sib, far) {
        (
            NodeSpan::Fixed {
                at: Anchor::Start(off),
                size,
            },
            true,
        ) => At::Start(i32::from(off) + i32::from(size) + by),
        (
            NodeSpan::Fixed {
                at: Anchor::Start(off),
                ..
            },
            false,
        ) => At::Start(i32::from(off) + by),
        (
            NodeSpan::Fixed {
                at: Anchor::End(off),
                ..
            },
            true,
        ) => At::End(i32::from(off) - by),
        (
            NodeSpan::Fixed {
                at: Anchor::End(off),
                size,
            },
            false,
        ) => At::End(i32::from(off) + i32::from(size) - by),
        (NodeSpan::Stretch { end, .. }, true) => At::End(i32::from(end) - by),
        (NodeSpan::Stretch { start, .. }, false) => At::Start(i32::from(start) + by),
        (
            NodeSpan::Fixed {
                at: Anchor::Centre(mid),
                size,
            },
            far,
        ) => {
            let left = i32::from(mid) - i32::from(size) / 2;
            At::Centre(if far { left + i32::from(size) } else { left } + by)
        }
    }))
}

fn i16_of(v: i32) -> Result<i16, ImportError> {
    i16::try_from(v).map_err(|_| bad(format!("{v} px is out of range")))
}

/// A position: a number (from the parent's start edge, or its far edge
/// under `Just*=RIGHT/BOTTOM`), or a sibling's edge (a point).
enum Pos {
    Num(i32),
    Ref(At),
}

/// A stretching tile from a point measured from the parent's far edge (its
/// far end `end` from that edge too) is fixed: xSLIMFIRE's tabs
/// (`X="TabNormalButton.RIGHT+12" ResizeX="100"`).
fn from_far(e: i32, start_in: i32, end: i32, axis: &Axis) -> Result<NodeSpan, ImportError> {
    let size = u16::try_from(e - start_in - end)
        .map_err(|_| bad(format!("{}: a stretch ending before it starts", axis.pos)))?;
    Ok(NodeSpan::Fixed {
        at: Anchor::End(i16_of(end)?),
        size,
    })
}

/// A stretch capped by its component's `MaxWidth`/`MaxHeight`: Xfire grows
/// the tile with its parent up to that size (xSLIMFIRE's
/// `TabNormalButton` `MaxWidth="65"`), so at every window it is that size.
fn capped(span: NodeSpan, max: Option<u16>, at_end: bool) -> NodeSpan {
    match (span, max) {
        (NodeSpan::Stretch { start, end }, Some(size)) => NodeSpan::Fixed {
            at: if at_end {
                Anchor::End(end)
            } else {
                Anchor::Start(start)
            },
            size,
        },
        (NodeSpan::Fixed { at, size }, Some(m)) => NodeSpan::Fixed {
            at,
            size: size.min(m),
        },
        (s, None) => s,
    }
}

pub(super) fn axis(
    tile: &Node,
    ax: &Axis,
    size: u16,
    max: Option<u16>,
    (sibs, sizes): (&Sibs, &Sizes),
) -> Result<NodeSpan, ImportError> {
    let just = tile.attr_ci(ax.just).map_or("", str::trim);
    let at_end = just.eq_ignore_ascii_case(ax.end_just);
    let centred = just.eq_ignore_ascii_case("CENTER") || just.eq_ignore_ascii_case("CENTRE");
    let pos = match tile.attr_ci(ax.pos) {
        Some(v) => match edge(v, sibs, ax)? {
            Some(at) => Pos::Ref(at),
            None => Pos::Num(expr(Some(v), (sibs, sizes), ax)?),
        },
        None => Pos::Num(0),
    };
    let ind = |k| expr(tile.attr_ci(k), (sibs, sizes), ax);
    let start_in = ind(ax.indent[0])? - ind(ax.outdent[0])?;
    let end_in = ind(ax.indent[1])? - ind(ax.outdent[1])?;
    if tile.attr_ci(ax.resize).is_some_and(|r| r.trim() == "100") {
        let (start, end) = match (pos, at_end) {
            (Pos::Num(p), true) => (start_in, p + end_in),
            (Pos::Num(p) | Pos::Ref(At::Start(p)), false) => (p + start_in, end_in),
            (Pos::Ref(At::End(far_at)), false) => {
                return Ok(capped(from_far(far_at, start_in, end_in, ax)?, max, at_end))
            }
            (Pos::Ref(At::Centre(_)), false) | (Pos::Ref(_), true) => {
                return Err(bad(format!(
                    "{}: ax stretch from an edge and ax far side",
                    ax.pos
                )))
            }
        };
        let span = NodeSpan::Stretch {
            start: i16_of(start)?,
            end: i16_of(end)?,
        };
        return Ok(capped(span, max, at_end));
    }
    let own = i32::from(size);
    // A sibling's edge is a point: `Just*=RIGHT/BOTTOM` puts the tile's far
    // edge on it (StealthX's `WndMax` `JustX=RIGHT X="WndClose.LEFT-3"`).
    let at = match (pos, at_end, centred) {
        (Pos::Num(p), _, true) => Anchor::Centre(i16_of(p)?),
        (Pos::Num(p), true, _) => Anchor::End(i16_of(p)?),
        (Pos::Num(p), false, _) if p < 0 => Anchor::End(i16_of(-p - own)?),
        (Pos::Num(p) | Pos::Ref(At::Start(p)), false, _) => Anchor::Start(i16_of(p)?),
        (Pos::Ref(At::Start(p)), true, _) => Anchor::Start(i16_of(p - own)?),
        (Pos::Ref(At::End(far_at)), true, _) => Anchor::End(i16_of(far_at)?),
        (Pos::Ref(At::End(far_at)), false, _) => Anchor::End(i16_of(far_at - own)?),
        (Pos::Ref(At::Centre(p)), true, _) => Anchor::Centre(i16_of(p - own / 2)?),
        (Pos::Ref(At::Centre(p)), false, _) => Anchor::Centre(i16_of(p + own / 2)?),
    };
    Ok(capped(NodeSpan::Fixed { at, size }, max, at_end))
}

/// `span` (in a parent's frame) in the frame of a child laid at `inner`,
/// when its edges are known there.
fn shift(span: NodeSpan, inner: NodeSpan) -> Option<NodeSpan> {
    let (start, end) = match inner {
        NodeSpan::Fixed {
            at: Anchor::Start(off),
            ..
        } => (Some(off), None),
        NodeSpan::Fixed {
            at: Anchor::End(off),
            ..
        } => (None, Some(off)),
        NodeSpan::Stretch { start, end } => (Some(start), Some(end)),
        NodeSpan::Fixed {
            at: Anchor::Centre(_),
            ..
        } => (None, None),
    };
    let by = |val: i16, off: Option<i16>| off.and_then(|off| val.checked_sub(off));
    Some(match span {
        NodeSpan::Fixed {
            at: Anchor::Start(off),
            size,
        } => NodeSpan::Fixed {
            at: Anchor::Start(by(off, start)?),
            size,
        },
        NodeSpan::Fixed {
            at: Anchor::End(off),
            size,
        } => NodeSpan::Fixed {
            at: Anchor::End(by(off, end)?),
            size,
        },
        NodeSpan::Stretch { start: lo, end: hi } => NodeSpan::Stretch {
            start: by(lo, start)?,
            end: by(hi, end)?,
        },
        NodeSpan::Fixed {
            at: Anchor::Centre(_),
            ..
        } => return None,
    })
}

/// The names a child of a tile laid at `(x, y)` sees: its parent's
/// siblings and their outer names (Total Chaos's `AdArea`
/// `Y="MainTitleBar.BOTTOM"` inside `AdBar`), in the child's frame.
pub(super) fn outer_for(names: &Sibs, x: NodeSpan, y: NodeSpan) -> Sibs {
    names
        .iter()
        .filter_map(|(k, (sx, sy))| Some((k.clone(), (shift(*sx, x)?, shift(*sy, y)?))))
        .collect()
}
