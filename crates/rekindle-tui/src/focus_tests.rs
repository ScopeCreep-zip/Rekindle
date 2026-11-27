use super::*;

#[test]
fn focus_ring_cycles_forward() {
    let mut ring = FocusRing::new(vec![
        FocusId::ChannelTree,
        FocusId::MessageList,
        FocusId::InputBox,
    ]);
    assert_eq!(ring.current(), FocusId::ChannelTree);
    ring.next();
    assert_eq!(ring.current(), FocusId::MessageList);
    ring.next();
    assert_eq!(ring.current(), FocusId::InputBox);
    ring.next();
    assert_eq!(ring.current(), FocusId::ChannelTree); // wraps
}

#[test]
fn focus_ring_cycles_backward() {
    let mut ring = FocusRing::new(vec![
        FocusId::ChannelTree,
        FocusId::MessageList,
        FocusId::InputBox,
    ]);
    ring.prev();
    assert_eq!(ring.current(), FocusId::InputBox); // wraps backward
    ring.prev();
    assert_eq!(ring.current(), FocusId::MessageList);
    ring.prev();
    assert_eq!(ring.current(), FocusId::ChannelTree);
}

#[test]
fn focus_ring_set_by_id() {
    let mut ring = FocusRing::new(vec![
        FocusId::ChannelTree,
        FocusId::MessageList,
        FocusId::InputBox,
    ]);
    ring.set(FocusId::InputBox);
    assert_eq!(ring.current(), FocusId::InputBox);
}

#[test]
fn focus_ring_set_unknown_id_is_noop() {
    let mut ring = FocusRing::new(vec![FocusId::ChannelTree, FocusId::MessageList]);
    ring.set(FocusId::PeerList); // not in ring
    assert_eq!(ring.current(), FocusId::ChannelTree); // unchanged
}

#[test]
fn focus_ring_is_focused() {
    let ring = FocusRing::new(vec![FocusId::ChannelTree, FocusId::MessageList]);
    assert!(ring.is_focused(FocusId::ChannelTree));
    assert!(!ring.is_focused(FocusId::MessageList));
}

#[test]
fn focus_ring_set_slots_preserves_focus() {
    let mut ring = FocusRing::new(vec![
        FocusId::ChannelTree,
        FocusId::MessageList,
        FocusId::InputBox,
    ]);
    ring.set(FocusId::MessageList);

    // Remove ChannelTree (sidebar hidden)
    ring.set_slots(vec![FocusId::MessageList, FocusId::InputBox]);
    assert_eq!(ring.current(), FocusId::MessageList); // preserved
}

#[test]
fn focus_ring_set_slots_resets_when_current_removed() {
    let mut ring = FocusRing::new(vec![
        FocusId::ChannelTree,
        FocusId::MessageList,
        FocusId::InputBox,
    ]);
    ring.set(FocusId::ChannelTree);

    // Remove ChannelTree
    ring.set_slots(vec![FocusId::MessageList, FocusId::InputBox]);
    assert_eq!(ring.current(), FocusId::MessageList); // reset to first
}

#[test]
fn focus_ring_len() {
    let ring = FocusRing::new(vec![FocusId::ChannelTree, FocusId::MessageList]);
    assert_eq!(ring.len(), 2);
    assert!(!ring.is_empty());
}

#[test]
#[should_panic(expected = "FocusRing requires at least one slot")]
fn focus_ring_empty_panics() {
    let _ring = FocusRing::new(vec![]);
}
