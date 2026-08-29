use super::*;

#[test]
fn auto_archive_accepts_spec_values() {
    for secs in [3600, 86_400, 259_200, 604_800] {
        assert_eq!(validate_auto_archive_seconds(secs).unwrap(), secs);
    }
}

#[test]
fn auto_archive_rejects_off_spec() {
    for secs in [0, 1, 60, 7200, 3_600_000, u64::MAX] {
        assert!(validate_auto_archive_seconds(secs).is_err(), "{secs}");
    }
}

#[test]
fn default_auto_archive_forum_post_is_seven_days() {
    assert_eq!(default_auto_archive_seconds("forum_post"), 604_800);
}

#[test]
fn default_auto_archive_announcement_is_three_days() {
    assert_eq!(default_auto_archive_seconds("announcement"), 259_200);
}

#[test]
fn default_auto_archive_text_is_one_day() {
    assert_eq!(default_auto_archive_seconds("text"), 86_400);
    assert_eq!(default_auto_archive_seconds("voice"), 86_400);
    assert_eq!(default_auto_archive_seconds("public"), 86_400);
    assert_eq!(default_auto_archive_seconds("unknown"), 86_400);
}

#[test]
fn is_thread_archived_manual_overrides_activity() {
    // archived_lamport=10, last_lamport=5 → manually archived
    assert!(is_thread_archived(Some(10), 5, 0, 86_400, 0));
}

#[test]
fn is_thread_archived_activity_after_archive_revives() {
    // archived_lamport=5, last_lamport=10 → revived
    assert!(!is_thread_archived(Some(5), 10, 0, 86_400, 0));
}

#[test]
fn is_thread_archived_auto_archives_after_window() {
    // last_activity=1000s, window=86400s, now=88400s → auto-archived
    assert!(is_thread_archived(None, 5, 1_000, 86_400, 88_500));
}

#[test]
fn is_thread_archived_never_archived_when_no_activity_no_archive() {
    // last_activity=0 (no messages yet) + no archived_lamport → live
    assert!(!is_thread_archived(None, 0, 0, 86_400, 1_000_000));
}

#[test]
fn member_count_is_segment_max() {
    assert_eq!(thread_member_count() as usize, MAX_MEMBERS_PER_SEGMENT);
}
