use super::*;

fn target(edit_seq: u64, edit_count: u32, deleted: bool) -> ChangeTarget {
    ChangeTarget {
        sender_key: "AAbb".into(),
        edit_seq,
        edit_count,
        deleted,
    }
}

#[test]
fn only_the_author_changes_its_message() {
    let t = target(0, 0, false);
    assert_eq!(
        judge_change(Some(&t), "ccdd", ChangeKind::Edit { seq: 4 }),
        Err(ChangeRefusal::NotAuthor)
    );
    assert_eq!(
        judge_change(Some(&t), "ccdd", ChangeKind::Delete),
        Err(ChangeRefusal::NotAuthor)
    );
    assert_eq!(
        judge_change(Some(&t), "aabb", ChangeKind::Edit { seq: 4 }),
        Ok(())
    );
    assert_eq!(
        judge_change(None, "aabb", ChangeKind::Delete),
        Err(ChangeRefusal::NoTarget)
    );
}

#[test]
fn the_latest_edit_by_seq_wins_up_to_the_cap() {
    assert_eq!(
        judge_change(
            Some(&target(7, 1, false)),
            "aabb",
            ChangeKind::Edit { seq: 7 }
        ),
        Err(ChangeRefusal::Stale)
    );
    assert_eq!(
        judge_change(
            Some(&target(7, MAX_EDITS, false)),
            "aabb",
            ChangeKind::Edit { seq: 9 }
        ),
        Err(ChangeRefusal::EditLimit)
    );
}

#[test]
fn our_own_change_is_judged_without_a_seq() {
    // Our edit's place is our stream's next seq: no seq refuses it.
    assert_eq!(
        judge_own_change(Some(&target(u64::MAX, 1, false)), "aabb", true),
        Ok(())
    );
    assert_eq!(
        judge_own_change(Some(&target(7, MAX_EDITS, false)), "aabb", true),
        Err(ChangeRefusal::EditLimit)
    );
    assert_eq!(
        judge_own_change(Some(&target(7, MAX_EDITS, false)), "aabb", false),
        Ok(())
    );
    assert_eq!(
        judge_own_change(Some(&target(0, 0, true)), "aabb", false),
        Err(ChangeRefusal::Deleted)
    );
    assert_eq!(
        judge_own_change(Some(&target(0, 0, false)), "ccdd", true),
        Err(ChangeRefusal::NotAuthor)
    );
}

#[test]
fn a_tombstone_is_final() {
    let t = target(7, MAX_EDITS, true);
    assert_eq!(
        judge_change(Some(&t), "aabb", ChangeKind::Edit { seq: 99 }),
        Err(ChangeRefusal::Deleted)
    );
    assert_eq!(
        judge_change(Some(&t), "aabb", ChangeKind::Delete),
        Err(ChangeRefusal::Deleted)
    );
    // A delete is allowed at any time before it, the cap reached or not.
    assert_eq!(
        judge_change(
            Some(&target(7, MAX_EDITS, false)),
            "aabb",
            ChangeKind::Delete
        ),
        Ok(())
    );
}
