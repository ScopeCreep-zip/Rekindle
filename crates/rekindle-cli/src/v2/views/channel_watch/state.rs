//! Channel watch view state.

use std::collections::{HashMap, HashSet};

use ratatui::layout::Rect;

use crate::v2::tui::components::channel_tree::ChannelTree;
use crate::v2::tui::components::input_box::InputBox;
use crate::v2::tui::components::message_list::MessageList;
use crate::v2::tui::components::peer_list::PeerList;
use crate::v2::tui::focus::{FocusId, FocusRing};
use crate::v2::tui::navigator::SplitPaneState;

pub const SIDEBAR_COLLAPSE_WIDTH: u16 = 60;
pub const PEER_LIST_COLLAPSE_WIDTH: u16 = 100;
pub const SIDEBAR_WIDTH: u16 = 22;
pub const PEER_LIST_WIDTH: u16 = 18;

pub struct ChannelWatchView {
    pub(super) community: String,
    pub(super) channel: String,
    pub(super) channel_id: Option<String>,
    pub(super) channel_tree: ChannelTree,
    pub(super) message_list: MessageList,
    pub(super) input_box: InputBox,
    pub(super) peer_list: PeerList,
    pub(super) focus: FocusRing,
    pub(super) sidebar_visible: bool,
    pub(super) terminal_width: u16,
    pub(super) typing_indicators: HashMap<String, std::time::Instant>,
    pub(super) pending_mek_requests: HashSet<u64>,
    pub(super) click_rects: HashMap<FocusId, Rect>,
    pub(super) split_dm: SplitPaneState,
    pub(super) split_dm_message_list: Option<MessageList>,
    pub(super) split_dm_input_box: Option<InputBox>,
}

impl ChannelWatchView {
    pub fn new(community: String, channel: String, use_unicode: bool) -> Self {
        Self {
            community: community.clone(),
            channel: channel.clone(),
            channel_id: None,
            channel_tree: ChannelTree::new(use_unicode),
            message_list: MessageList::new(community, channel),
            input_box: InputBox::new(),
            peer_list: PeerList::new(use_unicode),
            focus: FocusRing::new(vec![
                FocusId::ChannelTree, FocusId::MessageList,
                FocusId::InputBox, FocusId::PeerList,
            ]),
            sidebar_visible: true,
            terminal_width: 120,
            typing_indicators: HashMap::new(),
            pending_mek_requests: HashSet::new(),
            click_rects: HashMap::new(),
            split_dm: SplitPaneState::new(),
            split_dm_message_list: None,
            split_dm_input_box: None,
        }
    }

    pub fn community(&self) -> &str { &self.community }
    pub fn channel(&self) -> &str { &self.channel }

    pub fn channel_matches(&self, event_channel: &str) -> bool {
        event_channel == self.channel || self.channel_id.as_deref() == Some(event_channel)
    }

    pub fn update_focus_ring(&mut self) {
        let mut slots = Vec::new();
        if self.sidebar_visible && self.terminal_width >= SIDEBAR_COLLAPSE_WIDTH {
            slots.push(FocusId::ChannelTree);
        }
        slots.push(FocusId::MessageList);
        slots.push(FocusId::InputBox);
        if self.split_dm.active {
            slots.push(FocusId::SplitDmMessages);
            slots.push(FocusId::SplitDmInput);
        }
        if self.terminal_width >= PEER_LIST_COLLAPSE_WIDTH {
            slots.push(FocusId::PeerList);
        }
        self.focus.set_slots(slots);
    }

    pub fn expire_typing_indicators(&mut self) {
        let cutoff = std::time::Duration::from_secs(5);
        self.typing_indicators.retain(|_, instant| instant.elapsed() < cutoff);
    }

    pub fn typing_display(&self) -> Option<String> {
        let names = self.typing_names_internal();
        if names.is_empty() { None }
        else { Some(crate::v2::tui::components::typing_indicator::format_typing_compact(&names)) }
    }

    pub(crate) fn typing_names_internal(&self) -> Vec<String> {
        self.typing_indicators.keys()
            .map(|k| self.peer_list.resolve_name(k).unwrap_or_else(|| crate::v2::helpers::abbreviate_key(k)))
            .collect()
    }

    /// Open the split-pane DM for a peer clicked in the peer list.
    pub fn open_split_dm(&mut self, peer_key: &str) {
        let peer_name = self.peer_list.resolve_name(peer_key)
            .unwrap_or_else(|| crate::v2::helpers::abbreviate_key(peer_key));
        self.split_dm.toggle(peer_key.to_string(), peer_name);

        if self.split_dm.active {
            self.split_dm_message_list = Some(MessageList::new(String::new(), peer_key.to_string()));
            self.split_dm_input_box = Some(InputBox::new());
        } else {
            self.split_dm_message_list = None;
            self.split_dm_input_box = None;
        }
        self.update_focus_ring();
    }
}
