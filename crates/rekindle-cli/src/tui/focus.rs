//! Focus management for TUI views.
//!
//! [`FocusRing`] cycles focus between components within a view.
//! Tab advances, Shift+Tab reverses, mouse click sets directly.
//!
//! Source patterns:
//! - oxicord `chat_screen.rs` — ChatFocus enum with next()/previous()
//! - hypertile `core/state/focus.rs` — BSP tree path-based focus
//!
//! We use the simpler ring model (not BSP tree) because our layouts
//! are fixed three-pane, not dynamically tiled.

/// Identifier for a focusable component within a view.
///
/// Each view defines its own set of focusable components and creates
/// a FocusRing from them. The set may change based on layout state
/// (e.g., sidebar hidden → ChannelTree removed from ring).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusId {
    /// Sidebar channel/community tree.
    ChannelTree,
    /// Main message list.
    MessageList,
    /// Message input box.
    InputBox,
    /// Right sidebar peer/member list.
    PeerList,
    /// Doctor check list.
    DoctorList,
    /// Friend list.
    FriendList,
    /// DM conversation list.
    DmList,
    /// Voice participant list.
    VoiceParticipants,
    /// Community info panel.
    CommunityInfoPanel,
    /// Identity settings view.
    IdentitySettings,
}

/// A ring of focusable component IDs.
///
/// Cycles forward with `next()`, backward with `prev()`. Wraps at
/// both ends. The focused component draws its border in `accent.primary`;
/// all others use `border.unfocused`.
///
/// # Input capture rule
///
/// When the focused component is `InputBox` and the user is in input mode,
/// ALL key events are captured by the input box until Esc exits input mode.
/// The App checks `is_input_mode()` before dispatching to the keybinding system.
pub struct FocusRing {
    slots: Vec<FocusId>,
    current: usize,
}

impl FocusRing {
    /// Create a new focus ring with the given focusable components.
    ///
    /// The first slot is focused initially.
    /// Panics if `slots` is empty.
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

    /// Set focus to a specific component by ID.
    ///
    /// No-op if the ID is not in the ring (e.g., component is hidden).
    pub fn set(&mut self, id: FocusId) {
        if let Some(i) = self.slots.iter().position(|&f| f == id) {
            self.current = i;
        }
    }

    /// Replace the slot list (e.g., when layout changes hide/show panels).
    ///
    /// Preserves focus on the current ID if it still exists in the new set.
    /// Otherwise resets to the first slot.
    pub fn set_slots(&mut self, slots: Vec<FocusId>) {
        let prev_id = self.current();
        self.slots = slots;
        assert!(
            !self.is_empty(),
            "FocusRing requires at least one slot (len={})",
            self.len()
        );
        if let Some(i) = self.slots.iter().position(|&f| f == prev_id) {
            self.current = i;
        } else {
            self.current = 0;
        }
    }

    /// Number of focusable components in the ring.
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
        let mut ring = FocusRing::new(vec![
            FocusId::ChannelTree,
            FocusId::MessageList,
        ]);
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
}
