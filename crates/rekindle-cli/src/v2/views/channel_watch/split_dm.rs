//! Split-pane DM — right side of channel watch view.
//!
//! This module handles the split-pane DM lifecycle: opening when a peer
//! is clicked in the peer list, closing on Esc, and routing DM-specific
//! subscription events to the split pane's message list.
//!
//! The state (SplitPaneState, split_dm_message_list, split_dm_input_box)
//! lives on ChannelWatchView. This module provides the behavioral logic
//! that the layout, input, and events modules call into.

// The split-pane DM implementation is distributed across:
// - state.rs: open_split_dm(), SplitPaneState fields
// - layout.rs: render_split_dm_pane()
// - input.rs: SplitDmMessages/SplitDmInput focus handling
// - events.rs: DirectMessageReceived/TypingStarted/TypingStopped routing
//
// This file documents the integration and provides any standalone helpers.

use crate::v2::tui::action::Action;

use super::state::ChannelWatchView;

impl ChannelWatchView {
    /// Handle InputSubmit when the split DM input box is focused.
    pub fn handle_split_dm_submit(&mut self) -> Option<Action> {
        let ib = self.split_dm_input_box.as_mut()?;
        let text = ib.content();
        if text.trim().is_empty() || ib.is_over_limit() { return None; }

        let peer_key = self.split_dm.peer_key.clone();

        // Insert pending message into split DM message list
        if let Some(ref mut ml) = self.split_dm_message_list {
            let now = rekindle_utils::timestamp_ms();
            ml.push(rekindle_types::display::DecryptedMessageDisplay {
                message_id: format!("pending-{now}"),
                sequence: 0,
                author_pseudonym: String::new(),
                author_display_name: "you".to_string(),
                body: text.clone(),
                timestamp: now,
                reply_to_sequence: None,
                mek_generation: 0,
                is_encrypted: false,
                needs_mek: None,
                delivery_status: rekindle_types::display::DeliveryStatus::Sending,
            });
        }

        ib.clear();
        Some(Action::SendDm { peer_key, text })
    }
}
