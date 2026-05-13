//! Dashboard view state.

use std::collections::VecDeque;

use ratatui::layout::Rect;
use rekindle_types::display::{CommunityOverview, FriendDisplay};

use crate::v2::tui::focus::{FocusId, FocusRing};

/// Maximum data points retained for sparkline history.
const HISTORY_LEN: usize = 60;

pub struct DashboardView {
    pub(super) focus: FocusRing,
    pub(super) panel_rects: [Rect; 4],
    pub(super) identity_public_key: String,
    pub(super) identity_display_name: String,
    pub(super) node_attached: bool,
    pub(super) node_public_internet: bool,
    pub(super) node_uptime_secs: u64,
    pub(super) node_peer_count: usize,
    pub(super) node_route_allocated: bool,
    /// Peer count history for sparkline rendering (most recent at back).
    pub(super) peer_history: VecDeque<f64>,
    /// Community count history for area graph rendering.
    pub(super) community_history: VecDeque<f64>,
    pub(super) communities: Vec<CommunityOverview>,
    pub(super) friends: Vec<FriendDisplay>,
    pub(super) loaded: bool,
    pub(super) use_unicode: bool,
    pub(super) active_transfers: usize,
    pub(super) bytes_sent: u64,
    pub(super) bytes_received: u64,
}

impl DashboardView {
    pub fn new(use_unicode: bool) -> Self {
        Self {
            focus: FocusRing::new(vec![
                FocusId::DashIdentity,
                FocusId::DashNode,
                FocusId::ChannelTree,
                FocusId::FriendList,
            ]),
            panel_rects: [Rect::default(); 4],
            identity_public_key: String::new(),
            identity_display_name: String::new(),
            node_attached: false,
            node_public_internet: false,
            node_uptime_secs: 0,
            node_peer_count: 0,
            node_route_allocated: false,
            peer_history: VecDeque::with_capacity(HISTORY_LEN),
            community_history: VecDeque::with_capacity(HISTORY_LEN),
            communities: Vec::new(),
            friends: Vec::new(),
            loaded: false,
            use_unicode,
            active_transfers: 0,
            bytes_sent: 0,
            bytes_received: 0,
        }
    }

    pub fn set_identity(&mut self, public_key: &str, display_name: &str) {
        self.identity_public_key = public_key.to_string();
        self.identity_display_name = display_name.to_string();
    }
}
