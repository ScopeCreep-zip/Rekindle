use super::*;

fn test_msg(author: &str, body: &str, ts: u64) -> DecryptedMessageDisplay {
    DecryptedMessageDisplay {
        message_id: format!("msg-{ts}"),
        sequence: ts / 1000,
        author_pseudonym: author.to_string(),
        author_display_name: author.to_string(),
        body: body.to_string(),
        timestamp: ts,
        reply_to_message_id: None,
        mek_generation: 1,
        is_encrypted: false,
        needs_mek: None,
        automod_blurred: false,
        is_own: false,
        edited: false,
        removed_by: None,
    }
}

#[test]
fn auto_scroll_on_push_when_at_bottom() {
    let mut list = MessageList::new("com".into(), "gen".into());
    for i in 0..10 {
        list.push(test_msg("alice", &format!("msg {i}"), i * 60_000));
    }
    assert_eq!(list.list_state.selected(), Some(9));
    assert!(list.auto_scroll);
}

#[test]
fn auto_scroll_disengages_on_scroll_up() {
    let mut list = MessageList::new("com".into(), "gen".into());
    for i in 0..10 {
        list.push(test_msg("alice", &format!("msg {i}"), i * 60_000));
    }
    list.scroll_up();
    assert!(!list.auto_scroll);
}

#[test]
fn auto_scroll_reengages_on_scroll_to_bottom() {
    let mut list = MessageList::new("com".into(), "gen".into());
    for i in 0..10 {
        list.push(test_msg("alice", &format!("msg {i}"), i * 60_000));
    }
    list.scroll_up();
    assert!(!list.auto_scroll);

    list.scroll_to_bottom();
    assert!(list.auto_scroll);
    assert_eq!(list.list_state.selected(), Some(9));
}

#[test]
fn message_grouping_same_author_within_window() {
    let mut list = MessageList::new("com".into(), "gen".into());
    list.push(test_msg("alice", "first", 0));
    list.push(test_msg("alice", "second", 60_000)); // 1 min later
    list.push(test_msg("alice", "third", 120_000)); // 2 min later

    assert_eq!(list.messages[0].group, MessageGroup::Full);
    assert_eq!(list.messages[1].group, MessageGroup::Compact);
    assert_eq!(list.messages[2].group, MessageGroup::Compact);
}

#[test]
fn message_grouping_breaks_after_seven_minutes() {
    let mut list = MessageList::new("com".into(), "gen".into());
    list.push(test_msg("alice", "first", 0));
    list.push(test_msg("alice", "second", 8 * 60 * 1000)); // 8 min later

    assert_eq!(list.messages[0].group, MessageGroup::Full);
    assert_eq!(list.messages[1].group, MessageGroup::Full); // breaks
}

#[test]
fn message_grouping_breaks_on_different_author() {
    let mut list = MessageList::new("com".into(), "gen".into());
    list.push(test_msg("alice", "hello", 0));
    list.push(test_msg("bob", "hi", 60_000));

    assert_eq!(list.messages[0].group, MessageGroup::Full);
    assert_eq!(list.messages[1].group, MessageGroup::Full);
}

#[test]
fn set_messages_replaces_all() {
    let mut list = MessageList::new("com".into(), "gen".into());
    list.push(test_msg("alice", "old", 0));

    let new_msgs = vec![test_msg("bob", "new1", 1000), test_msg("bob", "new2", 2000)];
    list.set_messages(new_msgs);

    assert_eq!(list.len(), 2);
    assert!(list.auto_scroll);
    assert_eq!(list.list_state.selected(), Some(1));
}

#[test]
fn empty_list_reports_correctly() {
    let list = MessageList::new("com".into(), "gen".into());
    assert!(list.is_empty());
    assert_eq!(list.len(), 0);
}
