//! TUI view system — full-screen views for each major feature.
//!
//! Each view owns its components and implements the [`View`] trait.
//! The [`ViewRegistry`] manages view lifecycle, transitions, and
//! routing of events/actions to the active view.

pub mod activity;
pub mod channel_watch;
pub mod community_info;
pub mod dashboard;
pub mod dm_inbox;
pub mod doctor;
pub mod favourites;
pub mod friend_list;
pub mod group_dm;
pub mod identity_settings;
pub mod installed_games;
pub mod profile;
pub mod servers;
pub mod voice_session;

use anyhow::Result;

/// A panel's border: bold while its slot has focus.
pub fn focus_border(focus: &FocusRing, id: crate::focus::FocusId) -> ratatui::style::Style {
    let style = ratatui::style::Style::new();
    if focus.is_focused(id) {
        style.add_modifier(ratatui::style::Modifier::BOLD)
    } else {
        style
    }
}

/// Route a mouse click to the focus slot whose rect contains it.
///
/// Focuses that slot and, when it is the input box, returns the action
/// that enters input mode. `dm_inbox` and `channel_watch` had written
/// this identically; a third view would have written it a third time.
pub fn click_to_focus(
    focus: &mut crate::focus::FocusRing,
    click_rects: &std::collections::HashMap<crate::focus::FocusId, ratatui::layout::Rect>,
    column: u16,
    row: u16,
) -> Option<crate::action::Action> {
    for (&id, rect) in click_rects {
        if column >= rect.x
            && column < rect.x + rect.width
            && row >= rect.y
            && row < rect.y + rect.height
        {
            focus.set(id);
            if id == crate::focus::FocusId::InputBox {
                return Some(crate::action::Action::EnterInputMode);
            }
            return None;
        }
    }
    None
}
use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::action::{Action, CommandResult};
use crate::focus::FocusRing;
use crate::theme::ThemeManager;

/// Identifies the currently active view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewKind {
    Dashboard,
    IdentitySettings,
    ChannelWatch {
        community: String,
        channel: String,
    },
    DmInbox,
    DmThread {
        peer_key: String,
    },
    VoiceSession {
        community: String,
        channel: String,
    },
    FriendList,
    GroupDms,
    Doctor,
    CommunityInfo {
        community: String,
    },
    /// A friend's profile, or ours (`peer_key` `None`), plan E7.2.
    Profile {
        peer_key: Option<String>,
        name: String,
    },
    /// Our favourite game servers (plan E7.2).
    Favourites,
    /// The local activity feed (plan E7.15, TU1(a)).
    Activity,
    /// The games we can launch (plan E7.15 IV3).
    InstalledGames,
    /// Favourite, recent, friends' and communities' servers (plan E7.2 G3).
    Servers,
}

/// Trait for all TUI views.
pub trait View {
    /// Render the view into the given area.
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()>;

    /// Process an action. Return a chained action if needed.
    fn update(&mut self, action: Action) -> Result<Option<Action>>;

    /// Handle a completed async command result.
    fn on_command_result(&mut self, result: CommandResult) -> Result<()>;

    /// Handle a real-time subscription event from the daemon.
    ///
    /// Called by the reducer when `Action::SubscriptionEvent` is dispatched.
    /// Views match on events relevant to their context and ignore everything else.
    /// Default: no-op.
    fn on_subscription_event(
        &mut self,
        _event: &rekindle_types::subscription_events::SubscriptionEvent,
    ) -> Result<()> {
        Ok(())
    }

    /// Advance time-dependent state (animations, expiry).
    fn tick(&mut self) -> Result<()> {
        Ok(())
    }

    /// Forward a key event to whichever component is currently focused.
    ///
    /// Called by `App::process_action` for key events that aren't handled
    /// by the global keybinding system. The view checks its `FocusRing`,
    /// dispatches to the focused component's `handle_key()`, and returns
    /// the resulting `Action` (if any) for the App reducer to process.
    ///
    /// Default: no-op. Views that own interactive components override this.
    fn handle_focused_key(&mut self, _key: KeyEvent) -> Option<Action> {
        None
    }

    /// Handle a mouse click at the given terminal position.
    ///
    /// Views hit-test the click coordinates against their panel rects
    /// (stored during the last `draw()`) and set focus to the clicked
    /// component. Returns `Some(Action)` if the click produced a
    /// navigation action (e.g., clicking a community name navigates to it).
    ///
    /// Default: no-op. Views that have clickable regions override this.
    fn handle_click(&mut self, _column: u16, _row: u16) -> Option<Action> {
        None
    }

    /// Access the view's focus ring.
    fn focus_ring(&mut self) -> &mut FocusRing;
}

/// Manages view instances and transitions.
///
/// Views are created on demand and cached. Transitioning to a view
/// that was previously active reuses the cached instance (preserving
/// scroll position, selection, etc.).
pub struct ViewRegistry {
    current: ViewKind,
    dashboard: dashboard::DashboardView,
    identity_settings: Option<identity_settings::IdentitySettingsView>,
    channel_watch: Option<channel_watch::ChannelWatchView>,
    dm_inbox: Option<dm_inbox::DmInboxView>,
    voice_session: Option<voice_session::VoiceSessionView>,
    friend_list: Option<friend_list::FriendListView>,
    group_dms: Option<group_dm::GroupDmListView>,
    doctor_view: Option<doctor::DoctorView>,
    community_info: Option<community_info::CommunityInfoView>,
    profile: Option<profile::ProfileView>,
    favourites: Option<favourites::FavouritesView>,
    activity: Option<activity::ActivityView>,
    installed_games: Option<installed_games::InstalledGamesView>,
    servers: Option<servers::ServersView>,
}

impl ViewRegistry {
    /// Create a new registry with the dashboard as the initial view.
    pub fn new(use_unicode: bool) -> Self {
        Self {
            current: ViewKind::Dashboard,
            dashboard: dashboard::DashboardView::new(use_unicode),
            identity_settings: None,
            channel_watch: None,
            dm_inbox: None,
            voice_session: None,
            friend_list: None,
            group_dms: None,
            doctor_view: None,
            community_info: None,
            profile: None,
            favourites: None,
            activity: None,
            installed_games: None,
            servers: None,
        }
    }

    /// Current view kind.
    pub fn current_kind(&self) -> &ViewKind {
        &self.current
    }

    /// Transition to a new view. Creates the view if it doesn't exist.
    pub fn transition(&mut self, kind: ViewKind, use_unicode: bool) {
        match &kind {
            ViewKind::Dashboard => {}
            ViewKind::IdentitySettings => {
                if self.identity_settings.is_none() {
                    self.identity_settings =
                        Some(identity_settings::IdentitySettingsView::new(use_unicode));
                }
            }
            ViewKind::ChannelWatch { community, channel } => {
                let needs_create = self
                    .channel_watch
                    .as_ref()
                    .is_none_or(|v| v.community() != community || v.channel() != channel);
                if needs_create {
                    self.channel_watch = Some(channel_watch::ChannelWatchView::new(
                        community.clone(),
                        channel.clone(),
                        use_unicode,
                    ));
                }
            }
            ViewKind::DmInbox | ViewKind::DmThread { .. } => {
                if self.dm_inbox.is_none() {
                    self.dm_inbox = Some(dm_inbox::DmInboxView::new(use_unicode));
                }
            }
            ViewKind::VoiceSession { community, channel } => {
                self.voice_session = Some(voice_session::VoiceSessionView::new(
                    community.clone(),
                    channel.clone(),
                    use_unicode,
                ));
            }
            ViewKind::FriendList => {
                if self.friend_list.is_none() {
                    self.friend_list = Some(friend_list::FriendListView::new(use_unicode));
                }
            }
            ViewKind::GroupDms => {
                if self.group_dms.is_none() {
                    self.group_dms = Some(group_dm::GroupDmListView::new());
                }
            }
            ViewKind::Doctor => {
                if self.doctor_view.is_none() {
                    self.doctor_view = Some(doctor::DoctorView::new(use_unicode));
                }
            }
            ViewKind::CommunityInfo { community } => {
                let needs_create = self
                    .community_info
                    .as_ref()
                    .is_none_or(|v| v.community() != community);
                if needs_create {
                    self.community_info =
                        Some(community_info::CommunityInfoView::new(community.clone()));
                }
            }
            ViewKind::Profile { peer_key, name } => {
                let needs_create = self
                    .profile
                    .as_ref()
                    .is_none_or(|v| v.peer_key() != peer_key.as_deref());
                if needs_create {
                    self.profile = Some(profile::ProfileView::new(peer_key.clone(), name.clone()));
                }
            }
            ViewKind::Favourites => {
                if self.favourites.is_none() {
                    self.favourites = Some(favourites::FavouritesView::new());
                }
            }
            ViewKind::Activity => {
                if self.activity.is_none() {
                    self.activity = Some(activity::ActivityView::new());
                }
            }
            ViewKind::InstalledGames => {
                if self.installed_games.is_none() {
                    self.installed_games = Some(installed_games::InstalledGamesView::new());
                }
            }
            ViewKind::Servers => {
                if self.servers.is_none() {
                    self.servers = Some(servers::ServersView::new());
                }
            }
        }
        self.current = kind;
    }

    /// Direct access to the dashboard view for identity initialization.
    pub fn dashboard_mut(&mut self) -> &mut dashboard::DashboardView {
        &mut self.dashboard
    }

    /// Mutable reference to the current view.
    pub fn current_mut(&mut self) -> &mut dyn View {
        match &self.current {
            ViewKind::Dashboard => &mut self.dashboard,
            ViewKind::IdentitySettings => self
                .identity_settings
                .as_mut()
                .expect("identity_settings view should exist after transition"),
            ViewKind::ChannelWatch { .. } => self
                .channel_watch
                .as_mut()
                .expect("channel_watch view should exist after transition"),
            ViewKind::DmInbox | ViewKind::DmThread { .. } => self
                .dm_inbox
                .as_mut()
                .expect("dm_inbox view should exist after transition"),
            ViewKind::VoiceSession { .. } => self
                .voice_session
                .as_mut()
                .expect("voice_session view should exist after transition"),
            ViewKind::FriendList => self
                .friend_list
                .as_mut()
                .expect("friend_list view should exist after transition"),
            ViewKind::GroupDms => self
                .group_dms
                .as_mut()
                .expect("group_dms view should exist after transition"),
            ViewKind::Doctor => self
                .doctor_view
                .as_mut()
                .expect("doctor view should exist after transition"),
            ViewKind::CommunityInfo { .. } => self
                .community_info
                .as_mut()
                .expect("community_info view should exist after transition"),
            ViewKind::Profile { .. } => self
                .profile
                .as_mut()
                .expect("profile view should exist after transition"),
            ViewKind::Favourites => self
                .favourites
                .as_mut()
                .expect("favourites view should exist after transition"),
            ViewKind::Activity => self
                .activity
                .as_mut()
                .expect("activity view should exist after transition"),
            ViewKind::InstalledGames => self
                .installed_games
                .as_mut()
                .expect("installed_games view should exist after transition"),
            ViewKind::Servers => self
                .servers
                .as_mut()
                .expect("servers view should exist after transition"),
        }
    }
}
