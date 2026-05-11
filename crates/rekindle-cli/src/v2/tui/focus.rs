//! Focus management — ring-cycle between focusable components.
//!
//! Tab advances, Shift+Tab reverses, mouse click sets directly.
//! Slot list is dynamic — adapts when layout changes hide/show panels.

/// Identifier for a focusable component within a view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusId {
    ChannelTree,
    MessageList,
    InputBox,
    PeerList,
    DoctorList,
    FriendList,
    DmList,
    VoiceParticipants,
    CommunityInfoPanel,
    IdentitySettings,
    /// Split-pane DM message list (right side of channel watch).
    SplitDmMessages,
    /// Split-pane DM input box (right side of channel watch).
    SplitDmInput,
    /// Dashboard: identity panel (top-left).
    DashIdentity,
    /// Dashboard: node status panel (top-right).
    DashNode,
}

/// A ring of focusable component IDs with wrap-around navigation.
#[derive(Debug)]
pub struct FocusRing {
    slots: Vec<FocusId>,
    current: usize,
}

impl FocusRing {
    /// Create a new focus ring. Panics if `slots` is empty.
    pub fn new(slots: Vec<FocusId>) -> Self {
        assert!(!slots.is_empty(), "FocusRing requires at least one slot");
        Self { slots, current: 0 }
    }

    /// The currently focused component.
    pub fn current(&self) -> FocusId {
        self.slots[self.current]
    }

    /// Whether the given component is currently focused.
    pub fn is_focused(&self, id: FocusId) -> bool {
        self.current() == id
    }

    /// Advance focus to the next component. Wraps to first.
    pub fn next(&mut self) {
        self.current = (self.current + 1) % self.slots.len();
    }

    /// Move focus to the previous component. Wraps to last.
    pub fn prev(&mut self) {
        self.current = if self.current == 0 {
            self.slots.len() - 1
        } else {
            self.current - 1
        };
    }

    /// Set focus to a specific component. No-op if ID not in ring.
    pub fn set(&mut self, id: FocusId) {
        if let Some(i) = self.slots.iter().position(|&f| f == id) {
            self.current = i;
        }
    }

    /// Replace the slot list. Preserves focus if the current ID exists
    /// in the new set; otherwise resets to first slot.
    pub fn set_slots(&mut self, slots: Vec<FocusId>) {
        assert!(!slots.is_empty(), "FocusRing requires at least one slot");
        let prev_id = self.current();
        self.slots = slots;
        if let Some(i) = self.slots.iter().position(|&f| f == prev_id) {
            self.current = i;
        } else {
            self.current = 0;
        }
    }

    /// Number of focusable components.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Whether the ring is empty (should never be true after construction).
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_forward() {
        let mut ring = FocusRing::new(vec![FocusId::ChannelTree, FocusId::MessageList, FocusId::InputBox]);
        assert_eq!(ring.current(), FocusId::ChannelTree);
        ring.next();
        assert_eq!(ring.current(), FocusId::MessageList);
        ring.next();
        assert_eq!(ring.current(), FocusId::InputBox);
        ring.next();
        assert_eq!(ring.current(), FocusId::ChannelTree);
    }

    #[test]
    fn cycles_backward() {
        let mut ring = FocusRing::new(vec![FocusId::ChannelTree, FocusId::MessageList, FocusId::InputBox]);
        ring.prev();
        assert_eq!(ring.current(), FocusId::InputBox);
    }

    #[test]
    fn set_by_id() {
        let mut ring = FocusRing::new(vec![FocusId::ChannelTree, FocusId::MessageList, FocusId::InputBox]);
        ring.set(FocusId::InputBox);
        assert_eq!(ring.current(), FocusId::InputBox);
    }

    #[test]
    fn set_unknown_noop() {
        let mut ring = FocusRing::new(vec![FocusId::ChannelTree, FocusId::MessageList]);
        ring.set(FocusId::PeerList);
        assert_eq!(ring.current(), FocusId::ChannelTree);
    }

    #[test]
    fn set_slots_preserves_focus() {
        let mut ring = FocusRing::new(vec![FocusId::ChannelTree, FocusId::MessageList, FocusId::InputBox]);
        ring.set(FocusId::MessageList);
        ring.set_slots(vec![FocusId::MessageList, FocusId::InputBox]);
        assert_eq!(ring.current(), FocusId::MessageList);
    }

    #[test]
    fn set_slots_resets_when_removed() {
        let mut ring = FocusRing::new(vec![FocusId::ChannelTree, FocusId::MessageList, FocusId::InputBox]);
        ring.set(FocusId::ChannelTree);
        ring.set_slots(vec![FocusId::MessageList, FocusId::InputBox]);
        assert_eq!(ring.current(), FocusId::MessageList);
    }

    #[test]
    #[should_panic(expected = "FocusRing requires at least one slot")]
    fn empty_panics() {
        let _ring = FocusRing::new(vec![]);
    }
}
