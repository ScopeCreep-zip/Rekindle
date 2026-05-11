//! TUI view system — full-screen views for each major feature.
//!
//! Two traits enforce the read/write boundary at the type level:
//! - `ViewQuery` — read-only accessors (`&self`): typing indicators, message search index
//! - `View: ViewQuery` — mutable operations (`&mut self`): draw, update, event handling
//!
//! `ViewRegistry` exposes both `current_ref(&self) -> &dyn ViewQuery` and
//! `current_mut(&mut self) -> &mut dyn View`. Callers pick the accessor
//! that matches their borrow context — no unnecessary `&mut self` escalation.

pub mod channel_watch;
pub mod community_info;
pub mod dashboard;
pub mod dm_inbox;
pub mod dm_thread;
pub mod doctor;
pub mod file_preview;
pub mod friend_list;
pub mod identity_settings;
pub mod voice_session;

use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::Frame;

use crate::v2::tui::action::{Action, CommandResult};
use crate::v2::tui::focus::FocusRing;
use crate::v2::tui::theme::ThemeManager;

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
    FilePreview { path: String, line: Option<usize> },
}

/// Read-only view queries — accessible via `&self`.
pub trait ViewQuery {
    fn typing_names(&self) -> Vec<String> { Vec::new() }
    fn message_search_index(&self) -> Vec<(String, String, String)> { Vec::new() }
}

/// Mutable view operations — accessible via `&mut self`.
///
/// Methods return `Result` to propagate render and state errors to the
/// app loop, which logs them and continues. Views may call fallible
/// component methods (e.g., `input_box.draw()`, `frame.render_*`).
pub trait View: ViewQuery {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()>;
    fn update(&mut self, action: Action) -> Result<Option<Action>>;
    fn on_command_result(&mut self, result: CommandResult) -> Result<()>;
    fn on_subscription_event(&mut self, _event: &rekindle_types::subscription_events::SubscriptionEvent) -> Result<()> { Ok(()) }
    fn tick(&mut self) -> Result<()> { Ok(()) }
    fn handle_focused_key(&mut self, _key: KeyEvent) -> Option<Action> { None }
    fn handle_click(&mut self, _column: u16, _row: u16) -> Option<Action> { None }
    fn focus_ring(&mut self) -> &mut FocusRing;
}

/// Manages view instances — lazy instantiation, caching, transitions, event forwarding.
pub struct ViewRegistry {
    current: ViewKind,
    dashboard: dashboard::DashboardView,
    identity_settings: Option<identity_settings::IdentitySettingsView>,
    channel_watch: Option<channel_watch::ChannelWatchView>,
    dm_inbox: Option<dm_inbox::DmInboxView>,
    dm_thread: Option<dm_thread::DmThreadView>,
    voice_session: Option<voice_session::VoiceSessionView>,
    friend_list: Option<friend_list::FriendListView>,
    doctor_view: Option<doctor::DoctorView>,
    community_info: Option<community_info::CommunityInfoView>,
    file_preview: Option<file_preview::FilePreviewView>,
}

impl ViewRegistry {
    pub fn new(use_unicode: bool) -> Self {
        Self {
            current: ViewKind::Dashboard,
            dashboard: dashboard::DashboardView::new(use_unicode),
            identity_settings: None, channel_watch: None, dm_inbox: None,
            dm_thread: None, voice_session: None, friend_list: None,
            doctor_view: None, community_info: None, file_preview: None,
        }
    }

    pub fn current_kind(&self) -> &ViewKind { &self.current }

    pub fn transition(&mut self, kind: ViewKind, use_unicode: bool) {
        match &kind {
            ViewKind::Dashboard => {}
            ViewKind::IdentitySettings => {
                if self.identity_settings.is_none() {
                    self.identity_settings = Some(identity_settings::IdentitySettingsView::new(use_unicode));
                }
            }
            ViewKind::ChannelWatch { community, channel } => {
                let needs = self.channel_watch.as_ref()
                    .is_none_or(|v| v.community() != community || v.channel() != channel);
                if needs {
                    self.channel_watch = Some(channel_watch::ChannelWatchView::new(
                        community.clone(), channel.clone(), use_unicode,
                    ));
                }
            }
            ViewKind::DmInbox => {
                if self.dm_inbox.is_none() { self.dm_inbox = Some(dm_inbox::DmInboxView::new(use_unicode)); }
            }
            ViewKind::DmThread { ref peer_key } => {
                let needs = self.dm_thread.as_ref().is_none_or(|v| v.peer_key() != peer_key);
                if needs { self.dm_thread = Some(dm_thread::DmThreadView::new(peer_key.clone(), use_unicode)); }
            }
            ViewKind::VoiceSession { community, channel } => {
                self.voice_session = Some(voice_session::VoiceSessionView::new(
                    community.clone(), channel.clone(), use_unicode,
                ));
            }
            ViewKind::FriendList => {
                if self.friend_list.is_none() { self.friend_list = Some(friend_list::FriendListView::new(use_unicode)); }
            }
            ViewKind::Doctor => {
                if self.doctor_view.is_none() { self.doctor_view = Some(doctor::DoctorView::new(use_unicode)); }
            }
            ViewKind::CommunityInfo { community } => {
                let needs = self.community_info.as_ref().is_none_or(|v| v.community() != community);
                if needs { self.community_info = Some(community_info::CommunityInfoView::new(community.clone())); }
            }
            ViewKind::FilePreview { path, line } => {
                let needs = self.file_preview.as_ref().is_none_or(|v| v.file_path() != path);
                if needs { self.file_preview = Some(file_preview::FilePreviewView::new(path.clone(), *line)); }
            }
        }
        self.current = kind;
    }

    pub fn dashboard_mut(&mut self) -> &mut dashboard::DashboardView { &mut self.dashboard }

    pub fn forward_event_to_all(&mut self, event: &rekindle_types::subscription_events::SubscriptionEvent) -> Result<()> {
        self.dashboard.on_subscription_event(event)?;
        if let Some(ref mut v) = self.identity_settings { v.on_subscription_event(event)?; }
        if let Some(ref mut v) = self.channel_watch { v.on_subscription_event(event)?; }
        if let Some(ref mut v) = self.dm_inbox { v.on_subscription_event(event)?; }
        if let Some(ref mut v) = self.dm_thread { v.on_subscription_event(event)?; }
        if let Some(ref mut v) = self.voice_session { v.on_subscription_event(event)?; }
        if let Some(ref mut v) = self.friend_list { v.on_subscription_event(event)?; }
        if let Some(ref mut v) = self.doctor_view { v.on_subscription_event(event)?; }
        if let Some(ref mut v) = self.community_info { v.on_subscription_event(event)?; }
        Ok(())
    }

    pub fn current_ref(&self) -> &dyn ViewQuery {
        match &self.current {
            ViewKind::Dashboard => &self.dashboard,
            ViewKind::IdentitySettings => self.identity_settings.as_ref().expect("transitioned"),
            ViewKind::ChannelWatch { .. } => self.channel_watch.as_ref().expect("transitioned"),
            ViewKind::DmInbox => self.dm_inbox.as_ref().expect("transitioned"),
            ViewKind::DmThread { .. } => self.dm_thread.as_ref().expect("transitioned"),
            ViewKind::VoiceSession { .. } => self.voice_session.as_ref().expect("transitioned"),
            ViewKind::FriendList => self.friend_list.as_ref().expect("transitioned"),
            ViewKind::Doctor => self.doctor_view.as_ref().expect("transitioned"),
            ViewKind::CommunityInfo { .. } => self.community_info.as_ref().expect("transitioned"),
            ViewKind::FilePreview { .. } => self.file_preview.as_ref().expect("transitioned"),
        }
    }

    pub fn current_mut(&mut self) -> &mut dyn View {
        match &self.current {
            ViewKind::Dashboard => &mut self.dashboard,
            ViewKind::IdentitySettings => self.identity_settings.as_mut().expect("transitioned"),
            ViewKind::ChannelWatch { .. } => self.channel_watch.as_mut().expect("transitioned"),
            ViewKind::DmInbox => self.dm_inbox.as_mut().expect("transitioned"),
            ViewKind::DmThread { .. } => self.dm_thread.as_mut().expect("transitioned"),
            ViewKind::VoiceSession { .. } => self.voice_session.as_mut().expect("transitioned"),
            ViewKind::FriendList => self.friend_list.as_mut().expect("transitioned"),
            ViewKind::Doctor => self.doctor_view.as_mut().expect("transitioned"),
            ViewKind::CommunityInfo { .. } => self.community_info.as_mut().expect("transitioned"),
            ViewKind::FilePreview { .. } => self.file_preview.as_mut().expect("transitioned"),
        }
    }
}
