//! Dashboard input — 2x2 grid navigation, shortcut keys.

use crossterm::event::KeyCode;

use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::TerminalEvent;
use crate::v2::tui::focus::FocusId;
use crate::v2::tui::state::navigation::ViewKind;
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;

pub fn handle(
    event: &TerminalEvent,
    state: &mut TuiState,
    _caches: &mut RenderCaches,
) -> Vec<Effect> {
    let TerminalEvent::Key(key) = event else { return vec![]; };

    match key.code {
        KeyCode::Char('h') => {
            match state.nav.focus_ring.current() {
                FocusId::DashNode => state.nav.focus_ring.set(FocusId::DashIdentity),
                FocusId::FriendList => state.nav.focus_ring.set(FocusId::ChannelTree),
                _ => {}
            }
            vec![]
        }
        KeyCode::Char('l') => {
            match state.nav.focus_ring.current() {
                FocusId::DashIdentity => state.nav.focus_ring.set(FocusId::DashNode),
                FocusId::ChannelTree => state.nav.focus_ring.set(FocusId::FriendList),
                _ => {}
            }
            vec![]
        }
        KeyCode::Char('j') | KeyCode::Down => {
            match state.nav.focus_ring.current() {
                FocusId::DashIdentity => state.nav.focus_ring.set(FocusId::ChannelTree),
                FocusId::DashNode => state.nav.focus_ring.set(FocusId::FriendList),
                _ => {}
            }
            vec![]
        }
        KeyCode::Char('k') | KeyCode::Up => {
            match state.nav.focus_ring.current() {
                FocusId::ChannelTree => state.nav.focus_ring.set(FocusId::DashIdentity),
                FocusId::FriendList => state.nav.focus_ring.set(FocusId::DashNode),
                _ => {}
            }
            vec![]
        }
        KeyCode::Enter => {
            match state.nav.focus_ring.current() {
                FocusId::DashIdentity => vec![Effect::Navigate(ViewKind::IdentitySettings)],
                FocusId::DashNode => vec![Effect::Navigate(ViewKind::Doctor)],
                FocusId::ChannelTree => {
                    state.communities.list.first().map(|c| {
                        vec![Effect::Navigate(ViewKind::CommunityInfo { community: c.governance_key.clone() })]
                    }).unwrap_or_default()
                }
                FocusId::FriendList => vec![Effect::Navigate(ViewKind::FriendList)],
                _ => vec![],
            }
        }
        KeyCode::Char('d') => vec![Effect::Navigate(ViewKind::DmInbox)],
        KeyCode::Char('f') => vec![Effect::Navigate(ViewKind::FriendList)],
        _ => vec![],
    }
}
