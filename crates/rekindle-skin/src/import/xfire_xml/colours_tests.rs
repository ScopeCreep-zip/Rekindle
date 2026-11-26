use super::*;

#[test]
fn table_is_sorted_and_counts_hold() {
    assert!(NAMES.windows(2).all(|w| w[0].0 < w[1].0));
    assert_eq!(NAMES.len(), 73);
    let dropped = NAMES
        .iter()
        .filter(|(_, t)| matches!(t, Target::Dropped(_)))
        .count();
    assert_eq!(dropped, 23);
    assert_eq!(target("DisabledTextCG"), Target::NoRole("text.disabled"));
    assert_eq!(target("XIGEditModeTitleCG"), XIG);
}

#[test]
fn alpha_is_refused() {
    assert_eq!(parse_rgba("A", "RGBA(1,2,3,0)").unwrap(), [1, 2, 3]);
    assert_eq!(
        parse_rgba("A", "RGBA(-34,-11,57,0)").unwrap(),
        [-34, -11, 57]
    );
    assert!(matches!(
        parse_rgba("A", "RGBA(1,2,3,7)"),
        Err(ImportError::NonZeroAlpha { alpha: 7, .. })
    ));
    assert!(parse_rgba("A", "RGBA(1,2,300,0)").is_err());
    assert!(absolute("A", "v", [-1, 0, 0]).is_err());
}

fn attrs<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |k| {
        pairs
            .iter()
            .find(|(n, _)| *n == k)
            .map(|(_, v)| (*v).to_owned())
    }
}

#[test]
fn attribute_colours_take_the_same_alpha_rule() {
    let ok = [
        ("red", "-34"),
        ("green", "-11"),
        ("blue", "57"),
        ("alpha", "0"),
    ];
    assert_eq!(
        parse_attrs("A", attrs(&ok)).unwrap().unwrap(),
        [-34, -11, 57]
    );
    // Absent alpha: Classic v3's `AllServersText`, `ScrollbarColor`.
    let absent = [("red", "141"), ("green", "190"), ("blue", "255")];
    assert_eq!(
        parse_attrs("A", attrs(&absent)).unwrap().unwrap(),
        [141, 190, 255]
    );
    let alpha = [("red", "1"), ("green", "2"), ("blue", "3"), ("alpha", "9")];
    assert!(matches!(
        parse_attrs("A", attrs(&alpha)),
        Some(Err(ImportError::NonZeroAlpha { alpha: 9, .. }))
    ));
    let missing = [("red", "1"), ("blue", "3"), ("alpha", "0")];
    assert!(matches!(
        parse_attrs("A", attrs(&missing)),
        Some(Err(ImportError::BadColour { .. }))
    ));
    let wide = [("red", "1"), ("green", "256"), ("blue", "3")];
    assert!(parse_attrs("A", attrs(&wide)).unwrap().is_err());
    assert!(parse_attrs("A", attrs(&[])).is_none());
}
