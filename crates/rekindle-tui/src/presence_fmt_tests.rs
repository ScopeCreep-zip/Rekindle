use super::*;

#[test]
fn rank_orders_online_first_unknown_last() {
    let mut all = ["offline", "online", "mystery", "busy", "away"];
    all.sort_by_key(|s| presence_rank(s));
    assert_eq!(all, ["online", "away", "busy", "offline", "mystery"]);
}

/// Both glyph sets must cover every status, and the label must not
/// depend on the glyph set — the two widgets previously derived
/// these separately.
#[test]
fn indicator_label_is_independent_of_glyph_set() {
    for status in ["online", "away", "busy", "offline", "???"] {
        let (uni, uni_label) = presence_indicator(status, true);
        let (ascii, ascii_label) = presence_indicator(status, false);
        assert_eq!(uni_label, ascii_label, "label differs for {status}");
        assert!(!uni.is_empty() && !ascii.is_empty());
    }
}

#[test]
fn capitalize_handles_empty_and_unicode() {
    assert_eq!(capitalize_status(""), "");
    assert_eq!(capitalize_status("online"), "Online");
    assert_eq!(capitalize_status("état"), "État");
}
