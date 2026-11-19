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
fn is_thread_archived_standing_manual_archive_holds() {
    // A standing manual archive holds whatever the activity.
    assert!(is_thread_archived(true, 5, 86_400, 6));
}

#[test]
fn is_thread_archived_auto_archives_after_window() {
    // last_activity=1000s, window=86400s, now=88400s → auto-archived
    assert!(is_thread_archived(false, 1_000, 86_400, 88_500));
}

#[test]
fn is_thread_archived_never_archived_when_no_activity_no_archive() {
    // last_activity=0 (no messages yet) + no standing archive → live
    assert!(!is_thread_archived(false, 0, 86_400, 1_000_000));
}
