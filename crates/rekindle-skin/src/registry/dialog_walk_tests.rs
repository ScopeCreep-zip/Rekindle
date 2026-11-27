use super::*;
use crate::registry::dialog;

/// Lead round 4 c: the crossing retry moves only a dialog role. Text
/// darker than a mid-grey selection that black cannot clear crosses
/// to white; the pack's `accent.fill` and every other base token stay.
#[test]
fn the_crossing_retry_never_moves_a_base_token() {
    let mut t = BTreeMap::from([
        (
            "accent.fill".to_owned(),
            lightness::color([0x6a, 0x6a, 0x6a]),
        ),
        (
            "selection.text".to_owned(),
            lightness::color([0x50, 0x50, 0x50]),
        ),
    ]);
    let before = t.clone();
    let done = settle(&mut t, Base::Dark, &dialog::is_role);
    assert_eq!(done.unsettled, None);
    let moved = done
        .changes
        .iter()
        .find(|c| c.token == "selection.text")
        .expect("selection.text crosses");
    assert!(colour::luminance(moved.new) > colour::luminance(moved.original));
    for c in &done.changes {
        assert!(
            dialog::is_role(&c.token),
            "{} is not a dialog role",
            c.token
        );
    }
    for (id, v) in &before {
        assert!(dialog::is_role(id) || t.get(id) == Some(v), "{id} moved");
    }
    assert!(t
        .keys()
        .all(|id| before.contains_key(id) || dialog::is_role(id)));
}

fn role_test_tokens(text: [u8; 3], fill: [u8; 3]) -> BTreeMap<String, TokenValue> {
    BTreeMap::from([
        ("accent.fill".to_owned(), lightness::color(fill)),
        ("selection.text".to_owned(), lightness::color(text)),
    ])
}

fn chroma(c: [u8; 3]) -> f64 {
    let [_, a, b] = lightness::oklab(c);
    a.hypot(b)
}

/// Lead round 5 e: a role that passes by lightness at its own chroma
/// keeps that chroma and hue (only its Oklab lightness moves).
#[test]
fn a_role_that_passes_at_its_chroma_keeps_it() {
    let original = [0xa0, 0x30, 0x30];
    let mut t = role_test_tokens(original, [0x20, 0x20, 0x60]);
    let done = settle(&mut t, Base::Dark, &dialog::is_role);
    assert_eq!(done.unsettled, None);
    let moved = done
        .changes
        .iter()
        .find(|c| c.token == "selection.text")
        .expect("selection.text walks");
    let [_, a0, b0] = lightness::oklab(original);
    let [_, a1, b1] = lightness::oklab(moved.new);
    assert!(
        (a1 - a0).abs() < 0.01 && (b1 - b0).abs() < 0.01,
        "{:?} lost chroma: {a0:.3},{b0:.3} -> {a1:.3},{b1:.3}",
        moved.new
    );
    assert!(moved.after_x100.is_some_and(|r| r >= 450), "{moved:?}");
}

/// Lead round 5 e: winamp's `#00ff00` window text derives
/// `selection.text`; on the dark base's `accent.fill` #167bc0 no
/// lightness at that chroma clears 4.5:1 either way (the dark end
/// clamps to #000b00, 4.43:1), so the role loses chroma until one does.
/// Fail-without: with no chroma path the walk stops unsettled.
#[test]
fn a_role_no_lightness_settles_loses_chroma_until_one_does() {
    let green = [0x00, 0xff, 0x00];
    let fill = [0x16, 0x7b, 0xc0];
    let mut t = role_test_tokens(green, fill);
    let done = settle(&mut t, Base::Dark, &dialog::is_role);
    assert_eq!(done.unsettled, None, "{:?}", done.changes);
    let moved = done
        .changes
        .iter()
        .find(|c| c.token == "selection.text")
        .expect("selection.text walks");
    assert!(chroma(moved.new) < chroma(green), "{:?}", moved.new);
    assert!(colour::contrast(moved.new, fill) >= 4.5, "{:?}", moved.new);
}
