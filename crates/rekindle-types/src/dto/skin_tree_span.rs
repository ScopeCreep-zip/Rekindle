//! One axis of a skin node's box (plan SK2, SK17-3): the anchor a fixed or
//! shared span sits by and the span itself, split from [`super::skin_tree`]
//! (re-exported there).

use serde::{Deserialize, Serialize};

/// Where a fixed-size node sits along one axis of its parent's box, px.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "offset")]
pub enum Anchor {
    /// Its near side this far from the parent's start edge.
    Start(i16),
    /// Its far side this far before the parent's end edge.
    End(i16),
    /// Centred in its parent's box widened by `before` px at its start and
    /// `after` px at its end, then moved by `by` (plan SK2 lead decision
    /// 32): the frame the pack centred it in. A parent an `Outdent*` cut
    /// keeps its uncut length here (Total Chaos's `BottomCenter` tab,
    /// centred in the 471 px `BottomBar` that lost its first column); a
    /// client box or a capped stretch is a narrower frame (negative).
    /// Xfire floors: the start is ⌊(len + before + after − size) / 2⌋ +
    /// `by` − `before`, the one rule the shape and the stylesheet place by.
    Centre { by: i16, before: i16, after: i16 },
    /// Its near side `gap` px past the far side of `sibling`, a node of the
    /// same parent (plan SK15b): Xfire's `X="XfireMenuText.RIGHT+10"`
    /// (Onyx `mainwindow.xml:42`) chained on a content-sized text
    /// ([`ContentFit`]), which moves with that text's served width. A
    /// stored pack keeps it; the Host resolves every one into a [`Self::Start`]
    /// before it serves a tree (`rekindle_skin::skin_tree::content`), and
    /// [`super::skin_tree::SkinTree::boxes`] places it against its sibling's box.
    After { sibling: String, gap: i16 },
}

impl Anchor {
    /// Centred in the parent's own box, then moved by `by`.
    #[must_use]
    pub const fn centre(by: i16) -> Self {
        Self::Centre {
            by,
            before: 0,
            after: 0,
        }
    }
}

/// One axis of a node's box, parent-relative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", deny_unknown_fields)]
pub enum NodeSpan {
    /// Its own size (its art's), placed once.
    Fixed { at: Anchor, size: u16 },
    /// Stretched between two insets from the parent's edges (Xfire's
    /// `ResizeX="100"` with `Indent*`; negative for an `Outdent*`).
    Stretch { start: i16, end: i16 },
    /// A share of its parent (plan SK17-3, lead ruling): `permille`/1000
    /// of the parent's length on this axis, floored, placed by `at` as a
    /// [`Self::Fixed`] span of that size: Xfire's proportional
    /// `ResizeY="80"` (Combat's chat `ChatArea` is 80% of `ContentArea`,
    /// `CombatChatSkin.ini:291-299`). After a sibling ([`Anchor::After`])
    /// the parent's length is what remains past the span's start: Combat's
    /// `ChatEdit` (`Y=Divider.BOTTOM ResizeY=100 IndentBottom=0`) is all
    /// of the rest.
    Share {
        at: Anchor,
        permille: u16,
        /// Px held back before the parent's far edge (plan SKW3, lead
        /// ruling b). A share placed from its start ([`Anchor::Start`] or
        /// [`Anchor::After`]) is of what lies between its start and that
        /// point; any other is of the parent's length short of it.
        /// xfire-classic's chat (`xfirechatwindow.xml:211-238`): `ChatArea`
        /// `Y=ChatMenuBar.BOTTOM ResizeY=80 IndentBottom="19+20"` is 80% of
        /// the box from its start to 39 px above the bottom (the 19 px under
        /// the edit, its 14 px divider and 6 px of edit), and `ChatEdit`
        /// `Y=ChatDivider.BOTTOM ResizeY=100 IndentBottom=19` the rest less
        /// the 19 px under it. Combat's `ChatArea` (`Y=0`, no inset) is 80%
        /// of its whole area. Zero is left out of the JSON. After a
        /// sibling whose box moves with the window (SKC4c: centred, or
        /// anchored at the end) it runs from that sibling's laid edge:
        /// Bionical's chat `Rightlines` `X=SendButton.RIGHT ResizeX=100
        /// IndentRight=20` beside the centred `SendButton`
        /// (`biochat.ini:113-124`) is `After { SendButton, 0 }`, `permille`
        /// 1000, `end` 20.
        #[serde(default, skip_serializing_if = "is_zero")]
        end: i16,
    },
}

/// Whether a share holds no end inset (its JSON leaves `end` out).
const fn is_zero(end: &i16) -> bool {
    *end == 0
}

/// `permille`/1000 of `len`, floored: a share's size.
#[must_use]
pub fn share_of(len: i32, permille: u16) -> i32 {
    (len.max(0) * i32::from(permille)).div_euclid(1000)
}

/// Where a span `size` px long starts inside a parent `len` px long when
/// anchored by `at` (an [`Anchor::After`] as though its sibling ended at
/// the parent's start edge).
fn anchored(at: &Anchor, len: i32, size: i32) -> i32 {
    match at {
        Anchor::Start(o) | Anchor::After { gap: o, .. } => i32::from(*o),
        Anchor::End(o) => len - i32::from(*o) - size,
        Anchor::Centre { by, before, after } => {
            let (b, a) = (i32::from(*before), i32::from(*after));
            (len + b + a - size).div_euclid(2) + i32::from(*by) - b
        }
    }
}

impl NodeSpan {
    /// The span's start and length inside a parent `len` px long. An
    /// [`Anchor::After`] is placed as though its sibling ended at the
    /// parent's start edge: only [`super::skin_tree::SkinTree::boxes`]
    /// knows the sibling, and places it by [`Self::place_from`].
    #[must_use]
    pub fn place(&self, len: i32) -> (i32, i32) {
        match self {
            Self::Fixed { at, size } => {
                let size = i32::from(*size);
                (anchored(at, len, size), size)
            }
            Self::Stretch { start, end } => {
                let s = i32::from(*start);
                (s, len - s - i32::from(*end))
            }
            Self::Share {
                at: at @ Anchor::After { gap, .. },
                permille,
                end,
            } => (
                anchored(at, len, 0),
                share_of(len - i32::from(*gap) - i32::from(*end), *permille),
            ),
            Self::Share { at, permille, end } => {
                let start = match at {
                    Anchor::Start(o) => i32::from(*o),
                    _ => 0,
                };
                let size = share_of(len - start - i32::from(*end), *permille);
                (anchored(at, len, size), size)
            }
        }
    }

    /// An [`Anchor::After`] span's start and length in a parent `len` px
    /// long when its sibling's far edge lies `sib_end` px past the
    /// parent's start; `None` for any other span.
    #[must_use]
    pub fn place_from(&self, len: i32, sib_end: i32) -> Option<(i32, i32)> {
        let (_, gap) = self.after()?;
        let start = sib_end + i32::from(gap);
        Some(match self {
            Self::Share { permille, end, .. } => {
                (start, share_of(len - start - i32::from(*end), *permille))
            }
            _ => (start, self.place(len).1),
        })
    }

    /// `self` through `f`, a move of a fixed span's anchor (a cut, an
    /// outdent or a change of frame): a share goes through as the fixed
    /// span it lays at `len` and keeps its `permille` of the parent it
    /// then lies in, at the anchor `f` gives.
    #[must_use]
    pub fn share_through(self, len: i32, f: impl FnOnce(Self) -> Self) -> Self {
        let size = u16::try_from(self.place(len).1.max(0)).unwrap_or(u16::MAX);
        let Self::Share { at, permille, end } = self else {
            return f(self);
        };
        match f(Self::Fixed { at, size }) {
            Self::Fixed { at, .. } => Self::Share { at, permille, end },
            moved => moved,
        }
    }

    /// The sibling an [`Anchor::After`] span follows, if it is one.
    #[must_use]
    pub fn after(&self) -> Option<(&str, i16)> {
        match self {
            Self::Fixed {
                at: Anchor::After { sibling, gap },
                ..
            }
            | Self::Share {
                at: Anchor::After { sibling, gap },
                ..
            } => Some((sibling, *gap)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Anchor, NodeSpan};

    /// A share is `permille`/1000, floored, at the smallest and a 1200×900
    /// window's lengths: laid from its start (`Start(o)` or after a sibling)
    /// it takes its share of the box between that start and `end` px above
    /// the far edge (SKW3: xfire-classic's `ChatArea ResizeY=80
    /// IndentBottom="19+20"` only adds up this way); laid from the end it is
    /// its share of the whole parent, anchored there.
    #[test]
    fn a_share_places_at_its_permille_of_the_parent() {
        let share = |at| NodeSpan::Share {
            at,
            permille: 800,
            end: 0,
        };
        for (len, want) in [(161, 128), (900, 720), (1200, 960)] {
            assert_eq!(share(Anchor::Start(0)).place(len), (0, want), "{len}");
            assert_eq!(
                share(Anchor::Start(3)).place(len),
                (3, (len - 3) * 800 / 1000),
                "{len}"
            );
            assert_eq!(
                share(Anchor::End(4)).place(len),
                (len - 4 - want, want),
                "{len}"
            );
        }
        let rest = NodeSpan::Share {
            at: Anchor::After {
                sibling: "Divider".into(),
                gap: 2,
            },
            permille: 1000,
            end: 0,
        };
        assert_eq!(rest.place_from(900, 726), Some((728, 172)));
        let fixed = NodeSpan::Fixed {
            at: Anchor::After {
                sibling: "ChatArea".into(),
                gap: 0,
            },
            size: 6,
        };
        assert_eq!(fixed.place_from(900, 720), Some((720, 6)));
    }

    /// Plan SKW3 (lead ruling b): a share chained past a moving edge keeps
    /// its end inset. xfire-classic's chat `ChatEdit` (`Y=Divider.BOTTOM
    /// ResizeY=100 IndentBottom=19`) after a divider ending at 140 px in a
    /// 200 px area is the 41 px between them, and a share of the parent
    /// short of `end` is that share of the shorter length.
    #[test]
    fn a_share_keeps_its_end_inset() {
        let rest = NodeSpan::Share {
            at: Anchor::After {
                sibling: "Divider".into(),
                gap: 0,
            },
            permille: 1000,
            end: 19,
        };
        assert_eq!(rest.place_from(200, 140), Some((140, 41)));
        assert_eq!(rest.place(200), (0, 181));
        let share = NodeSpan::Share {
            at: Anchor::Start(0),
            permille: 800,
            end: 20,
        };
        assert_eq!(share.place(220), (0, 160));
        let moved = share.share_through(220, |f| f);
        assert_eq!(moved.place(220), (0, 160));
        let chat_area = NodeSpan::Share {
            at: Anchor::Start(27),
            permille: 800,
            end: 39,
        };
        assert_eq!(chat_area.place(270), (27, 163));
    }
}
