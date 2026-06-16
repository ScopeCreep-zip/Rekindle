//! View dispatch — draw, input, and required_data routing by ViewKind.

pub mod channel_watch;
pub mod community_info;
pub mod dashboard;
pub mod dm_inbox;
pub mod dm_thread;
pub mod doctor;
pub mod events_calendar;
pub mod file_preview;
pub mod friend_list;
pub mod identity_settings;
pub mod invite;
pub mod moderation;
pub mod onboarding;
pub mod voice_session;

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::TerminalEvent;
use crate::v2::tui::state::navigation::ViewKind;
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;
use crate::v2::tui::theme::ThemeManager;

pub fn draw(
    state: &TuiState,
    frame: &mut Frame,
    area: Rect,
    theme: &ThemeManager,
    caches: &mut RenderCaches,
) {
    match state.nav.current_view() {
        ViewKind::Dashboard => dashboard::draw::draw(state, frame, area, theme, caches),
        ViewKind::IdentitySettings => identity_settings::draw::draw(state, frame, area, theme, caches),
        ViewKind::ChannelWatch { community, channel } => {
            channel_watch::draw::draw(state, frame, area, theme, community, channel, caches)
        }
        ViewKind::DmInbox => dm_inbox::draw::draw(state, frame, area, theme, caches),
        ViewKind::DmThread { peer_key } => {
            dm_thread::draw::draw(state, frame, area, theme, peer_key, caches)
        }
        ViewKind::VoiceSession { community, channel } => {
            voice_session::draw::draw(state, frame, area, theme, community, channel, caches)
        }
        ViewKind::FriendList => friend_list::draw::draw(state, frame, area, theme, caches),
        ViewKind::Doctor => doctor::draw::draw(state, frame, area, theme, caches),
        ViewKind::CommunityInfo { community } => {
            community_info::draw::draw(state, frame, area, theme, community, caches)
        }
        ViewKind::Moderation { community } => {
            moderation::draw::draw(state, frame, area, theme, community, caches)
        }
        ViewKind::Invite { community } => {
            invite::draw::draw(state, frame, area, theme, community, caches)
        }
        ViewKind::Events { community } => {
            events_calendar::draw::draw(state, frame, area, theme, community, caches)
        }
        ViewKind::Onboarding { community } => {
            onboarding::draw::draw(state, frame, area, theme, community, caches)
        }
        ViewKind::FilePreview { path, line } => {
            file_preview::draw::draw(state, frame, area, theme, path, *line, caches)
        }
    }
}

pub fn dispatch_view_input(
    view: &ViewKind,
    event: &TerminalEvent,
    state: &mut TuiState,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    match view {
        ViewKind::Dashboard => dashboard::input::handle(event, state, caches),
        ViewKind::IdentitySettings => identity_settings::input::handle(event, state, caches),
        ViewKind::ChannelWatch { community, channel } => {
            channel_watch::input::handle(event, state, community, channel, caches)
        }
        ViewKind::DmInbox => dm_inbox::input::handle(event, state, caches),
        ViewKind::DmThread { peer_key } => {
            dm_thread::input::handle(event, state, peer_key, caches)
        }
        ViewKind::VoiceSession { community, channel } => {
            voice_session::input::handle(event, state, community, channel, caches)
        }
        ViewKind::FriendList => friend_list::input::handle(event, state, caches),
        ViewKind::Doctor => doctor::input::handle(event, state, caches),
        ViewKind::CommunityInfo { community } => {
            community_info::input::handle(event, state, community, caches)
        }
        ViewKind::Moderation { community } => {
            moderation::input::handle(event, state, community, caches)
        }
        ViewKind::Invite { community } => {
            invite::input::handle(event, state, community, caches)
        }
        ViewKind::Events { community } => {
            events_calendar::input::handle(event, state, community, caches)
        }
        ViewKind::Onboarding { community } => {
            onboarding::input::handle(event, state, community, caches)
        }
        ViewKind::FilePreview { path, line } => {
            file_preview::input::handle(event, state, path, *line, caches)
        }
    }
}
