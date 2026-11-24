//! Where a cloned tab goes in its strip ([`super::place`]'s
//! `Tab(Communities)`, plan SK3): one pitch past the last tab, or, for a
//! content-sized tab (plan SK15b), the strip's gap past the last tab's
//! served width.

use rekindle_types::dto::skin_tree::{Anchor, ContentFit, NodeSpan};
use rekindle_types::dto::skin_tree_role::ViewKey;

/// The span one pitch past `last` (the distance from `prev`'s anchor, or
/// `last`'s size with no `prev`), on the same edge.
pub(super) fn step(last: NodeSpan, prev: Option<NodeSpan>) -> NodeSpan {
    let NodeSpan::Fixed { at, size } = last else {
        return last;
    };
    let pitch =
        |a: i16, p: Option<i16>| p.map_or_else(|| i16::try_from(size).unwrap_or(0), |p| a - p);
    let at = match (at, prev) {
        (
            Anchor::Start(a),
            Some(NodeSpan::Fixed {
                at: Anchor::Start(p),
                ..
            }),
        ) => Anchor::Start(a + pitch(a, Some(p))),
        (Anchor::Start(a), _) => Anchor::Start(a + pitch(a, None)),
        (
            Anchor::End(a),
            Some(NodeSpan::Fixed {
                at: Anchor::End(p), ..
            }),
        ) => Anchor::End(a + pitch(a, Some(p))),
        (Anchor::End(a), _) => Anchor::End(a - pitch(a, None)),
        (other, _) => other,
    };
    NodeSpan::Fixed { at, size }
}

/// A content-sized tab's clone (plan SK15b, Simple-X's `TabHeader` words
/// chained `TabServerText.RIGHT+10`): served "Communities" wide, after
/// `last` by the gap `prev` leaves before it, not the equal-width pitch of
/// [`step`]; `stepped` where the strip is not laid from its start.
pub(super) fn follow(
    last: &NodeSpan,
    prev: Option<&NodeSpan>,
    stepped: NodeSpan,
    fit: ContentFit,
) -> NodeSpan {
    let size = super::content::width(fit, ViewKey::Communities.title());
    match (last, prev, stepped) {
        (
            NodeSpan::Fixed {
                at: Anchor::Start(a),
                size: s,
            },
            Some(NodeSpan::Fixed {
                at: Anchor::Start(p),
                size: ps,
            }),
            _,
        ) => {
            let (a, s) = (i32::from(*a), i32::from(*s));
            let at = a + s + a - i32::from(*p) - i32::from(*ps);
            NodeSpan::Fixed {
                at: Anchor::Start(i16::try_from(at).unwrap_or(i16::MAX)),
                size,
            }
        }
        (_, _, NodeSpan::Fixed { at, .. }) => NodeSpan::Fixed { at, size },
        (_, _, other) => other,
    }
}
