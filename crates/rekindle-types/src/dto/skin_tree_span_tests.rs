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
