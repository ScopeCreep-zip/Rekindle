//! Split-pane DM state — right side of channel watch view.
//!
//! When a user clicks a member in the peer list, the center message
//! area splits horizontally: channel messages on the left, DM thread
//! with the selected member on the right. Both panes independently
//! receive and process SubscriptionEvents.

use super::super::focus::{FocusId, FocusRing};

/// Split-pane DM state tracked by the channel watch view.
#[derive(Debug)]
pub struct SplitPaneState {
    /// Whether the split pane is currently active.
    pub active: bool,
    /// The peer key of the member being DM'd.
    pub peer_key: String,
    /// The peer's display name.
    pub peer_name: String,
    /// Whether the peer is currently typing.
    pub peer_typing: bool,
    /// Focus ring for the split pane (SplitDmMessages, SplitDmInput).
    pub focus: FocusRing,
}

impl SplitPaneState {
    /// Create a new inactive split pane.
    pub fn new() -> Self {
        Self {
            active: false,
            peer_key: String::new(),
            peer_name: String::new(),
            peer_typing: false,
            focus: FocusRing::new(vec![FocusId::SplitDmMessages, FocusId::SplitDmInput]),
        }
    }

    /// Open the split pane for a specific peer.
    pub fn open(&mut self, peer_key: String, peer_name: String) {
        self.active = true;
        self.peer_key = peer_key;
        self.peer_name = peer_name;
        self.peer_typing = false;
        self.focus = FocusRing::new(vec![FocusId::SplitDmMessages, FocusId::SplitDmInput]);
    }

    /// Close the split pane.
    pub fn close(&mut self) {
        self.active = false;
        self.peer_key.clear();
        self.peer_name.clear();
        self.peer_typing = false;
    }

    /// Toggle the split pane — close if active for this peer, open if different.
    pub fn toggle(&mut self, peer_key: String, peer_name: String) {
        if self.active && self.peer_key == peer_key {
            self.close();
        } else {
            self.open(peer_key, peer_name);
        }
    }

    /// Whether the split pane DM matches this peer.
    pub fn is_peer(&self, peer_key: &str) -> bool {
        self.active && self.peer_key == peer_key
    }
}

impl Default for SplitPaneState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_inactive() {
        let state = SplitPaneState::new();
        assert!(!state.active);
        assert!(state.peer_key.is_empty());
    }

    #[test]
    fn open_activates() {
        let mut state = SplitPaneState::new();
        state.open("pk1".into(), "alice".into());
        assert!(state.active);
        assert_eq!(state.peer_key, "pk1");
        assert_eq!(state.peer_name, "alice");
    }

    #[test]
    fn close_deactivates() {
        let mut state = SplitPaneState::new();
        state.open("pk1".into(), "alice".into());
        state.close();
        assert!(!state.active);
        assert!(state.peer_key.is_empty());
    }

    #[test]
    fn toggle_same_peer_closes() {
        let mut state = SplitPaneState::new();
        state.open("pk1".into(), "alice".into());
        state.toggle("pk1".into(), "alice".into());
        assert!(!state.active);
    }

    #[test]
    fn toggle_different_peer_switches() {
        let mut state = SplitPaneState::new();
        state.open("pk1".into(), "alice".into());
        state.toggle("pk2".into(), "bob".into());
        assert!(state.active);
        assert_eq!(state.peer_key, "pk2");
    }

    #[test]
    fn is_peer_check() {
        let mut state = SplitPaneState::new();
        state.open("pk1".into(), "alice".into());
        assert!(state.is_peer("pk1"));
        assert!(!state.is_peer("pk2"));
    }
}
