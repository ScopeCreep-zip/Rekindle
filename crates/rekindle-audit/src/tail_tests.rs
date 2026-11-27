use super::*;

#[test]
fn an_anchor_behind_the_store_is_a_catch_up() {
    assert_eq!(
        TailCheck::of(Some((3, [0xAA; MAC_LEN])), (5, [0xBB; MAC_LEN])),
        TailCheck::CatchUp
    );
}

#[test]
fn an_anchor_ahead_of_the_store_is_tampering() {
    let anchor = (5, [0xAA; MAC_LEN]);
    let check = TailCheck::of(Some(anchor), (3, [0xBB; MAC_LEN]));
    assert_eq!(check, TailCheck::Tampered { anchor });
    assert_eq!(check.resume_from((3, [0xBB; MAC_LEN])), anchor);
}

#[test]
fn the_same_cursor_with_another_mac_is_tampering() {
    let anchor = (5, [0xAA; MAC_LEN]);
    assert_eq!(
        TailCheck::of(Some(anchor), (5, [0xCC; MAC_LEN])),
        TailCheck::Tampered { anchor }
    );
}

#[test]
fn an_equal_tail_is_clean_and_no_anchor_is_unanchored() {
    let tail = (5, [0xAA; MAC_LEN]);
    assert_eq!(TailCheck::of(Some(tail), tail), TailCheck::Clean);
    assert_eq!(TailCheck::of(None, tail), TailCheck::Unanchored);
}
