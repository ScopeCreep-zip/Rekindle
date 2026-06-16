//! TuiState — all data owned by the state machine.

pub mod channels;
pub mod communities;
pub mod confirm;
pub mod dm;
pub mod emoji_picker;
pub mod ephemeral;
pub mod file_search;
pub mod friends;
pub mod in_flight;
pub mod navigation;
pub mod presence;
pub mod render_caches;
pub mod search;
pub mod session;
pub mod tab_bar;
pub mod tracked_buffer;
pub mod voice;

use std::time::Instant;

use self::channels::ChannelDataState;
use self::communities::CommunityState;
use self::dm::DmState;
use self::ephemeral::EphemeralState;
use self::friends::FriendState;
use self::in_flight::InFlightState;
use self::navigation::NavigationState;
use self::presence::PresenceState;
use self::session::SessionState;
use self::voice::VoiceState;
use super::idle::IdleTier;

pub struct TuiState {
    pub dm: DmState,
    pub channels: ChannelDataState,
    pub friends: FriendState,
    pub communities: CommunityState,
    pub presence: PresenceState,
    pub voice: VoiceState,
    pub nav: NavigationState,
    pub session: SessionState,
    pub ephemeral: EphemeralState,
    pub in_flight: InFlightState,
    pub idle: IdleTier,
    /// Client-side timezone display preference. Persisted across restarts.
    pub timezone: crate::v2::helpers::TimezoneMode,
    /// Single time source per select iteration.
    pub now: Instant,
    /// Milliseconds since Unix epoch.
    pub wall_clock_ms: u64,
    pub render_needed: bool,
    pub should_quit: bool,
    pub node_connected: bool,
    pub cached_peer_count: usize,
    pub status_snapshot: Option<rekindle_types::display::StatusSnapshot>,
    pub search_engine: Option<crate::v2::search::RekindleSearch>,
    /// Transient theme override for command palette preview.
    /// Set when hovering a theme item, cleared on search close.
    pub theme_preview: Option<String>,
}

impl TuiState {
    pub fn new(animated: bool, unicode: bool, now: Instant, wall_clock_ms: u64) -> Self {
        Self {
            dm: DmState::new(),
            channels: ChannelDataState::new(),
            friends: FriendState::new(),
            communities: CommunityState::new(),
            presence: PresenceState::new(),
            voice: VoiceState::new(),
            nav: NavigationState::new(),
            session: SessionState::new(),
            ephemeral: EphemeralState::new(animated, unicode, now),
            in_flight: InFlightState::new(),
            idle: IdleTier::Active,
            timezone: crate::v2::helpers::TimezoneMode::Local,
            now,
            wall_clock_ms,
            render_needed: true,
            should_quit: false,
            node_connected: false,
            cached_peer_count: 0,
            status_snapshot: None,
            search_engine: None,
            theme_preview: None,
        }
    }
}
