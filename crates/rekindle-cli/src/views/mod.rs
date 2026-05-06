//! TUI view system — full-screen views for each major feature.
//!
//! Each view owns its components and implements the [`View`] trait.
//! The [`ViewRegistry`] manages view lifecycle, transitions, and
//! routing of events/actions to the active view.

pub mod channel_watch;
pub mod community_info;
pub mod dashboard;
pub mod dm_inbox;
pub mod doctor;
pub mod friend_list;
pub mod identity_settings;
pub mod voice_session;

use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::tui::action::{Action, CommandResult};
use crate::tui::focus::FocusRing;
use crate::tui::theme::ThemeManager;

/// Identifies the currently active view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewKind {
    Dashboard,
    IdentitySettings,
    ChannelWatch { community: String, channel: String },
    DmInbox,
    DmThread { peer_key: String },
    VoiceSession { community: String, channel: String },
    FriendList,
    Doctor,
    CommunityInfo { community: String },
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
    doctor_view: Option<doctor::DoctorView>,
    community_info: Option<community_info::CommunityInfoView>,
    /// Action sender cloned to each view for async command spawning.
    /// Views use this to fire background loads (refresh, search, history pagination)
    /// without routing through the App reducer.
    #[allow(dead_code)] // M3 passes to views via init()
    action_tx: Option<tokio::sync::mpsc::UnboundedSender<Action>>,
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
            doctor_view: None,
            community_info: None,
            action_tx: None,
        }
    }

    /// Set the action sender for async command spawning from views.
    ///
    /// Called by `App::new()` after constructing the registry. M3 views
    /// use this sender to spawn background loads directly instead of
    /// routing every async request through the App reducer.
    #[allow(dead_code)] // M3 calls from App::new
    pub fn set_action_tx(&mut self, tx: tokio::sync::mpsc::UnboundedSender<Action>) {
        self.action_tx = Some(tx);
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
                    self.identity_settings = Some(identity_settings::IdentitySettingsView::new(use_unicode));
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
            ViewKind::Doctor => self
                .doctor_view
                .as_mut()
                .expect("doctor view should exist after transition"),
            ViewKind::CommunityInfo { .. } => self
                .community_info
                .as_mut()
                .expect("community_info view should exist after transition"),
        }
    }
}
