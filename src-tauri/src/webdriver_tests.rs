use super::parse_damage_next_post;

#[test]
fn only_one_arms_the_damaged_post_switch() {
    assert_eq!(parse_damage_next_post("1"), Ok(true));
    for bad in ["", "0", "true", "yes", " 1", "1 ", "2"] {
        assert!(parse_damage_next_post(bad).is_err(), "{bad:?} refused");
    }
}
