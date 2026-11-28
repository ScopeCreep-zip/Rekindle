use super::{Anchor, NodeSpan};

/// A share is `permille`/1000 of its parent, floored, at the smallest
/// and a 1200×900 window's lengths, anchored as a fixed span of that
/// size; after a sibling it takes its share of what remains.
#[test]
fn a_share_places_at_its_permille_of_the_parent() {
    let share = |at| NodeSpan::Share { at, permille: 800 };
    for (len, want) in [(161, 128), (900, 720), (1200, 960)] {
        assert_eq!(share(Anchor::Start(0)).place(len), (0, want), "{len}");
        assert_eq!(share(Anchor::Start(3)).place(len), (3, want), "{len}");
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
