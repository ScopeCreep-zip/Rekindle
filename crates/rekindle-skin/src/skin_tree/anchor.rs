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
use super::sibs::span_of;
pub(super) use super::sibs::Sibs;

/// One axis's attribute names.
pub(super) struct Axis {
    pub(super) pos: &'static str,
    just: &'static str,
    resize: &'static str,
    pub(super) indent: [&'static str; 2],
    pub(super) outdent: [&'static str; 2],
    far: &'static str,
    near: &'static str,
    size: &'static str,
    end_just: &'static str,
}

impl Axis {
    /// The axis's position attribute (`X`, `Y`), naming it in errors.
    pub(super) const fn pos(&self) -> &'static str {
        self.pos
    }
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
#[derive(Clone, Copy)]
pub(super) enum At {
    Start(i32),
    End(i32),
    Centre(i32),
}

/// Each sibling's art size, known before it is placed: XGlow's `Middle`
/// ends `Bottom.height` above the bottom while `Bottom` sits on
/// `Middle.BOTTOM`, a cycle only in position, never in size.
pub(super) type Sizes = BTreeMap<String, (u16, u16)>;

/// A tile's `Resize*` on `ax`, percent of its parent: `Some(100)` a
/// stretch, `Some(1..=99)` a share (plan SK17-3: Combat's chat `ChatArea`
/// `ResizeY=80`); `None` for none or 0 (its own size).
pub(super) fn resize(tile: &Node, ax: &Axis) -> Option<u16> {
    tile.attr_ci(ax.resize)
        .and_then(|r| r.trim().parse::<u16>().ok())
        .filter(|r| (1..=100).contains(r))
}

/// A sum of signed terms, each a number or a sibling's `width`/`height`.
pub(super) fn expr(
    v: Option<&str>,
    (sibs, sizes): (&Sibs, &Sizes),
    axis: &Axis,
) -> Result<i32, ImportError> {
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

/// `v` as a sibling's edge ± n, when it is one: a point, or the far edge
/// of a sibling that moves with the parent's length (a share, or a tile
/// chained past one: Combat's chat `Divider` `Y=ChatArea.BOTTOM`), which
/// the tile chains on.
fn edge(value: &str, sibs: &Sibs, axis: &Axis) -> Result<Option<Pos>, ImportError> {
    let Some((name, side, by)) = reference(value) else {
        return Ok(None);
    };
    if side.eq_ignore_ascii_case(axis.size) {
        return Ok(None);
    }
    let far = side.eq_ignore_ascii_case(axis.far);
    // A sibling's `CENTER` is the midpoint of its two edges (Combat's chat
    // `MainTitleBar` `X=LeftEdge.CENTER` then `X=60`,
    // `CombatChatSkin.ini:121-122`), when both move together.
    let centre = side.eq_ignore_ascii_case("CENTER") || side.eq_ignore_ascii_case("CENTRE");
    if !far && !centre && !side.eq_ignore_ascii_case(axis.near) {
        return Err(bad(format!("{value}: {side} is not an edge of this axis")));
    }
    let key = name.to_ascii_lowercase();
    if let (Some(sib), true, false) = (span_of(sibs, name, axis), far, centre) {
        if matches!(sib, NodeSpan::Share { .. }) || sib.after().is_some() {
            return Ok(Some(Pos::After(name.trim().to_owned(), by)));
        }
    }
    let at_edge = |far: bool| match span_of(sibs, name, axis) {
        Some(sib) => points(&sib)[usize::from(far)]
            .ok_or_else(|| bad(format!("{value}: {name}'s edge moves with the window"))),
        None => sibs
            .edges
            .get(&key)
            .and_then(|e| e[usize::from(axis.pos == "Y")][usize::from(far)])
            .ok_or_else(|| bad(format!("{value}: no tile {name} placed before it"))),
    };
    let at = if centre {
        match (at_edge(false)?, at_edge(true)?) {
            (At::Start(a), At::Start(b)) => At::Start((a + b).div_euclid(2)),
            (At::End(a), At::End(b)) => At::End((a + b).div_euclid(2)),
            (At::Centre(a), At::Centre(b)) => At::Centre((a + b).div_euclid(2)),
            _ => {
                return Err(bad(format!(
                    "{value}: {name}'s centre moves with the window"
                )))
            }
        }
    } else {
        at_edge(far)?
    };
    Ok(Some(Pos::Ref(match at {
        At::Start(p) => At::Start(p + by),
        At::End(e) => At::End(e - by),
        At::Centre(c) => At::Centre(c + by),
    })))
}

/// A box's near and far edge as points in its frame, where they are
/// points: a share's far edge (and a chained tile's edges) move with the
/// parent's length ([`edge`] chains on them).
fn points(span: &NodeSpan) -> [Option<At>; 2] {
    match span.clone() {
        NodeSpan::Fixed {
            at: Anchor::Start(off),
            size,
        } => [
            Some(At::Start(i32::from(off))),
            Some(At::Start(i32::from(off) + i32::from(size))),
        ],
        NodeSpan::Fixed {
            at: Anchor::End(off),
            size,
        } => [
            Some(At::End(i32::from(off) + i32::from(size))),
            Some(At::End(i32::from(off))),
        ],
        NodeSpan::Stretch { start, end } => [
            Some(At::Start(i32::from(start))),
            Some(At::End(i32::from(end))),
        ],
        NodeSpan::Share {
            at: Anchor::Start(off),
            ..
        } => [Some(At::Start(i32::from(off))), None],
        NodeSpan::Share {
            at: Anchor::End(off),
            ..
        } => [None, Some(At::End(i32::from(off)))],
        NodeSpan::Fixed {
            at: Anchor::After { .. },
            ..
        }
        | NodeSpan::Share { .. } => [None, None],
        NodeSpan::Fixed {
            at:
                Anchor::Centre {
                    by: mid,
                    before,
                    after,
                },
            size,
        } => {
            // A point is measured from the parent's centre: the frame's
            // widening moves the centre by half its difference.
            let mid = i32::from(mid) + (i32::from(after) - i32::from(before)).div_euclid(2);
            let left = mid - i32::from(size) / 2;
            [
                Some(At::Centre(left)),
                Some(At::Centre(left + i32::from(size))),
            ]
        }
    }
}

/// The edges of the frame of a child laid at `inner` that are known in its
/// parent's frame: its start from the start, its end from the end.
fn known(inner: &NodeSpan) -> (Option<i16>, Option<i16>) {
    match inner.clone() {
        NodeSpan::Fixed {
            at: Anchor::Start(off),
            ..
        }
        | NodeSpan::Share {
            at: Anchor::Start(off),
            ..
        } => (Some(off), None),
        NodeSpan::Fixed {
            at: Anchor::End(off),
            ..
        }
        | NodeSpan::Share {
            at: Anchor::End(off),
            ..
        } => (None, Some(off)),
        NodeSpan::Stretch { start, end } => (Some(start), Some(end)),
        NodeSpan::Fixed {
            at: Anchor::Centre { .. } | Anchor::After { .. },
            ..
        }
        | NodeSpan::Share { .. } => (None, None),
    }
}

/// A point of the parent's frame in the frame of a child laid at `inner`,
/// when the child's edge it is measured from is known there.
fn into_frame(at: At, inner: &NodeSpan) -> Option<At> {
    let (start, end) = known(inner);
    match at {
        At::Start(p) => start.map(|s| At::Start(p - i32::from(s))),
        At::End(e) => end.map(|o| At::End(e - i32::from(o))),
        At::Centre(_) => None,
    }
}

/// The sibling, offset and edge (`true` far, `false` near) of a tile
/// placed at a sibling's edge on `ax` (`X="XfireMenuText.RIGHT+10"`,
/// `Y="UserName.BOTTOM+2"`; XGlow's `Status` at `Y="StatusMiddle.TOP+1"`,
/// `Xglow.ini:216-217`), neither stretched nor justified to its end or
/// centre: a chain [`super::content::chains`] may keep symbolic.
pub(super) fn chain<'t>(tile: &'t Node, ax: &Axis) -> Option<(&'t str, i32, bool)> {
    let just = tile.attr_ci(ax.just).map_or("", str::trim);
    let placed_from_start = !just.eq_ignore_ascii_case(ax.end_just)
        && !just.eq_ignore_ascii_case("CENTER")
        && !just.eq_ignore_ascii_case("CENTRE");
    let stretched = resize(tile, ax).is_some();
    let (name, side, by) = reference(tile.attr_ci(ax.pos)?)?;
    let far = side.eq_ignore_ascii_case(ax.far);
    (placed_from_start && !stretched && (far || side.eq_ignore_ascii_case(ax.near))).then_some((
        name.trim(),
        by,
        far,
    ))
}

pub(super) fn i16_of(v: i32) -> Result<i16, ImportError> {
    i16::try_from(v).map_err(|_| bad(format!("{v} px is out of range")))
}

/// A position: a number (from the parent's start edge, or its far edge
/// under `Just*=RIGHT/BOTTOM`), or a sibling's edge (a point).
pub(super) enum Pos {
    Num(i32),
    Ref(At),
    /// `by` px past the far edge of the named sibling, which moves with
    /// the parent's length.
    After(String, i32),
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
/// `Just*=CENTER` centres it in the span it would stretch over (Simple X's
/// `AdArea`: `JustX=CENTER ResizeX=100 OutdentLeft=7 IndentRight=7` under
/// `MaxWidth=240`, `SimpleX.ini:121-127,729-732`: the ad box centred over
/// the frame left of the 13 px info strip, as the original shows).
fn capped(span: NodeSpan, max: Option<u16>, (at_end, centred): (bool, bool)) -> NodeSpan {
    match (span, max) {
        (NodeSpan::Stretch { start, end }, Some(size)) => NodeSpan::Fixed {
            at: if centred {
                Anchor::Centre {
                    by: 0,
                    before: start.saturating_neg(),
                    after: end.saturating_neg(),
                }
            } else if at_end {
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
        // [`axis`] refuses a capped share before it lays one.
        (s @ NodeSpan::Share { .. }, Some(_)) | (s, None) => s,
    }
}

/// `tile`'s box on `ax`. `root` is a tile laid directly in its window: a
/// bare negative number there is measured from the window's far edge, as a
/// tile cannot lie outside its window (Alloy's `InfoViewButton` `X=-59`,
/// `Infoskin.ini:29`; Onyx's `CloseInfoView` `X="-52"`,
/// `popups.xml:17,35,61`). Inside a nested tile (a title bar, a menu bar)
/// it is an offset before the tile's start: Combat's chat `Logo` `X=-30`
/// `Y=-3` under `[MainTitleBar]`, then `Title` `X=Logo.RIGHT X=20`
/// (`CombatChatSkin.ini:219-226`), the 48×23 mark hanging off the bar's
/// top-left corner as the original draws it.
pub(super) fn axis(
    tile: &Node,
    ax: &Axis,
    size: u16,
    max: Option<u16>,
    (slot, root): (bool, bool),
    (sibs, sizes): (&Sibs, &Sizes),
) -> Result<NodeSpan, ImportError> {
    let just = tile.attr_ci(ax.just).map_or("", str::trim);
    let at_end = just.eq_ignore_ascii_case(ax.end_just);
    let centred = just.eq_ignore_ascii_case("CENTER") || just.eq_ignore_ascii_case("CENTRE");
    let pos = match tile.attr_ci(ax.pos) {
        Some(v) => match edge(v, sibs, ax)? {
            Some(pos) => pos,
            None => Pos::Num(expr(Some(v), (sibs, sizes), ax)?),
        },
        None => Pos::Num(0),
    };
    let ind = |k| expr(tile.attr_ci(k), (sibs, sizes), ax);
    let start_in = ind(ax.indent[0])? - ind(ax.outdent[0])?;
    let end_in = ind(ax.indent[1])? - ind(ax.outdent[1])?;
    let pos = match pos {
        Pos::After(sibling, by) => {
            return super::anchor_share::chained(
                (sibling, by),
                size,
                resize(tile, ax),
                (start_in, end_in, max),
                (at_end, centred),
                ax,
            )
        }
        pos => pos,
    };
    if let Some(pct @ 1..=99) = resize(tile, ax) {
        return super::anchor_share::share(
            pos,
            pct,
            (start_in, end_in, max),
            (at_end, centred),
            ax,
        );
    }
    if resize(tile, ax) == Some(100) {
        let (start, end) = match (pos, at_end) {
            (Pos::Num(p), true) => (start_in, p + end_in),
            (Pos::Num(p) | Pos::Ref(At::Start(p)), false) => (p + start_in, end_in),
            (Pos::Ref(At::End(far_at)), false) => {
                return Ok(capped(
                    from_far(far_at, start_in, end_in, ax)?,
                    max,
                    (at_end, centred),
                ))
            }
            (Pos::Ref(At::Centre(_)) | Pos::After(..), false)
            | (Pos::Ref(_) | Pos::After(..), true) => {
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
        return Ok(capped(span, max, (at_end, centred)));
    }
    // An artless slot (a box the shell mounts into, `slot`) placed from
    // the start with an end indent spans the two: no art gives it a
    // width, its anchors do (xSLIMFIRE's
    // `ChatTypingMessage` `X="18" IndentRight="65"`,
    // `xSLIMFIREchatwindow.xml:31`: the typing line from 18 px to 65 px
    // short of the base board's right edge, left of its send button).
    // Without an end indent, an artless slot's line runs to the parent's
    // right edge, as a `Stretch=ToContent` text drew its string rightward
    // from its point (Alloy's `TypingMessage` `X=UserNameText.RIGHT X=5`,
    // `Chatskin.ini:207-210,222-231`; XGlow's `X=14`, `ChatSkin.ini:
    // 163-166,178-188`); its height stays the line box (`text_box`).
    // Any other artless tile keeps its own place and the text's width
    // (xfire-classic's `TabDownloadText` holds "Downloads").
    let ends = tile.attr_ci(ax.indent[1]).is_some() || ax.pos == X.pos;
    if let (true, 0, false, false, true, Pos::Num(p) | Pos::Ref(At::Start(p))) =
        (slot, size, at_end, centred, ends, &pos)
    {
        if *p >= 0 {
            let span = NodeSpan::Stretch {
                start: i16_of(p + start_in)?,
                end: i16_of(end_in)?,
            };
            return Ok(capped(span, max, (at_end, centred)));
        }
    }
    let own = i32::from(size);
    // A sibling's edge is a point: `Just*=RIGHT/BOTTOM` puts the tile's far
    // edge on it (StealthX's `WndMax` `JustX=RIGHT X="WndClose.LEFT-3"`).
    let at = match (pos, at_end, centred) {
        (Pos::Num(p), _, true) => Anchor::centre(i16_of(p)?),
        (Pos::Num(p), true, _) => Anchor::End(i16_of(p)?),
        (Pos::Num(p), false, _) if p < 0 && root => Anchor::End(i16_of(-p - own)?),
        (Pos::Num(p) | Pos::Ref(At::Start(p)), false, _) => Anchor::Start(i16_of(p)?),
        (Pos::Ref(At::Start(p)), true, _) => Anchor::Start(i16_of(p - own)?),
        (Pos::Ref(At::End(far_at)), true, _) => Anchor::End(i16_of(far_at)?),
        (Pos::Ref(At::End(far_at)), false, _) => Anchor::End(i16_of(far_at - own)?),
        (Pos::Ref(At::Centre(p)), true, _) => Anchor::centre(i16_of(p - own / 2)?),
        (Pos::Ref(At::Centre(p)), false, _) => Anchor::centre(i16_of(p + own / 2)?),
        (Pos::After(..), _, _) => return Err(bad(format!("{}: a chain laid twice", ax.pos))),
    };
    Ok(capped(NodeSpan::Fixed { at, size }, max, (at_end, centred)))
}

/// `span` (in a parent's frame) in the frame of a child laid at `inner`,
/// when its edges are known there.
fn shift(span: &NodeSpan, inner: &NodeSpan) -> Option<NodeSpan> {
    let (start, end) = known(inner);
    let span = span.clone();
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
        NodeSpan::Share {
            at: Anchor::Start(off),
            permille,
            end: held,
        } => NodeSpan::Share {
            at: Anchor::Start(by(off, start)?),
            permille,
            end: held,
        },
        NodeSpan::Share {
            at: Anchor::End(off),
            permille,
            end: held,
        } => NodeSpan::Share {
            at: Anchor::End(by(off, end)?),
            permille,
            end: held,
        },
        NodeSpan::Share { .. }
        | NodeSpan::Fixed {
            at: Anchor::Centre { .. } | Anchor::After { .. },
            ..
        } => return None,
    })
}

/// The names a child of a tile laid at `(x, y)` sees: its parent's
/// siblings and their outer names (Total Chaos's `AdArea`
/// `Y="MainTitleBar.BOTTOM"` inside `AdBar`), in the child's frame.
pub(super) fn outer_for(names: &Sibs, x: &NodeSpan, y: &NodeSpan) -> Sibs {
    let frame = [x, y];
    let moved = |pts: [[Option<At>; 2]; 2]| {
        let m = [0, 1].map(|a| pts[a].map(|p| p.and_then(|p| into_frame(p, frame[a]))));
        m.iter().flatten().any(Option::is_some).then_some(m)
    };
    let mut out = Sibs::new();
    for (k, (sx, sy)) in &names.spans {
        if let (Some(a), Some(b)) = (shift(sx, x), shift(sy, y)) {
            out.spans.insert(k.clone(), (a, b));
        } else if let Some(e) = moved([points(sx), points(sy)]) {
            out.edges.insert(k.clone(), e);
        }
    }
    for (k, e) in &names.edges {
        if let Some(e) = moved(*e) {
            out.edges.insert(k.clone(), e);
        }
    }
    out
}
