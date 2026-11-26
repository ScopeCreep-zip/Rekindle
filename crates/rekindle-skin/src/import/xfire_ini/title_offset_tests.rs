use super::offset_of;

/// Plan SK2 lead decision 4: XGlow's repeated position keys.
#[test]
fn a_repeated_position_key_is_an_offset_from_the_first_edge() {
    assert_eq!(
        offset_of("Y", "Bottom.BOTTOM", "-52").as_deref(),
        Some("Bottom.BOTTOM-52")
    );
    assert_eq!(
        offset_of("X", "ButtonDivider.LEFT", "62").as_deref(),
        Some("ButtonDivider.LEFT+62")
    );
    assert_eq!(
        offset_of("Y", "Middle.BOTTOM", "0").as_deref(),
        Some("Middle.BOTTOM+0")
    );
    assert_eq!(offset_of("Y", "Top", "13").as_deref(), Some("Top.TOP+13"));
    assert_eq!(offset_of("X", "Logo", "-2").as_deref(), Some("Logo.LEFT-2"));
    assert_eq!(offset_of("Y", "12", "0"), None);
    assert_eq!(offset_of("Z", "Bottom.BOTTOM", "1"), None);
}

/// A number before the edge is the edge plus it: Combat's chat `[Top]`
/// `X=0` then `X=TopLeft.RIGHT` (`CombatChatSkin.ini:26,30`).
#[test]
fn a_number_before_an_edge_offsets_that_edge() {
    let mut attrs = std::collections::BTreeMap::new();
    super::put(&mut attrs, "X", "0");
    super::put(&mut attrs, "X", "TopLeft.RIGHT");
    assert_eq!(attrs["X"], "TopLeft.RIGHT+0");
    super::put(&mut attrs, "Y", "Middle.BOTTOM");
    super::put(&mut attrs, "Y", "-7");
    assert_eq!(attrs["Y"], "Middle.BOTTOM-7");
}
