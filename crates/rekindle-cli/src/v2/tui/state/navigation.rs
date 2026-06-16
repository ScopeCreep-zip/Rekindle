//! Navigation state — view stack, input mode, overlays, sidebar, focus.

use std::collections::HashMap;

use crate::v2::tui::focus::{FocusId, FocusRing};

use super::confirm::ConfirmState;
use super::emoji_picker::EmojiPickerState;
use super::file_search::FileSearchState;
use super::search::SearchState;
use super::tab_bar::{Tab, TabBarState};

/// Identifies the active view. Every view in the TUI has a variant.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
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
    Moderation { community: String },
    Invite { community: String },
    Events { community: String },
    Onboarding { community: String },
    FilePreview { path: String, line: Option<usize> },
}

/// Pure discriminant of `ViewKind` — no data fields. Cheap to hash/compare.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ViewKindTag {
    Dashboard,
    IdentitySettings,
    ChannelWatch,
    DmInbox,
    DmThread,
    VoiceSession,
    FriendList,
    Doctor,
    CommunityInfo,
    Moderation,
    Invite,
    Events,
    Onboarding,
    FilePreview,
}

impl ViewKind {
    pub fn tag(&self) -> ViewKindTag {
        match self {
            Self::Dashboard => ViewKindTag::Dashboard,
            Self::IdentitySettings => ViewKindTag::IdentitySettings,
            Self::ChannelWatch { .. } => ViewKindTag::ChannelWatch,
            Self::DmInbox => ViewKindTag::DmInbox,
            Self::DmThread { .. } => ViewKindTag::DmThread,
            Self::VoiceSession { .. } => ViewKindTag::VoiceSession,
            Self::FriendList => ViewKindTag::FriendList,
            Self::Doctor => ViewKindTag::Doctor,
            Self::CommunityInfo { .. } => ViewKindTag::CommunityInfo,
            Self::Moderation { .. } => ViewKindTag::Moderation,
            Self::Invite { .. } => ViewKindTag::Invite,
            Self::Events { .. } => ViewKindTag::Events,
            Self::Onboarding { .. } => ViewKindTag::Onboarding,
            Self::FilePreview { .. } => ViewKindTag::FilePreview,
        }
    }
}

/// What the input box is doing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputContext {
    Compose,
    Reply { message_id: String, author: String },
    Edit { message_id: String, original: String },
    Search,
    SplitDmCompose { peer_key: String },
    ThreadCompose { thread_id: String },
}

/// Modal overlay on top of the view.
#[derive(Clone, Debug)]
pub enum OverlayState {
    Help,
    Confirm,
    EmojiPicker,
}

/// All navigation state.
#[derive(Debug)]
pub struct NavigationState {
    /// Dashboard is always at index 0.
    view_stack: Vec<ViewKind>,
    pub input_mode: bool,
    pub input_context: InputContext,
    pub overlay: Option<OverlayState>,
    pub confirm: ConfirmState,
    pub emoji_picker: EmojiPickerState,
    pub search: Option<SearchState>,
    pub file_search: Option<FileSearchState>,
    pub sidebar_visible: bool,
    pub tab_bar: TabBarState,
    /// Per-tab unread counts.
    pub tab_unreads: HashMap<String, u32>,
    pub focus_ring: FocusRing,
    pub terminal_focused: bool,
    /// Row index for tab bar click hit testing.
    pub tab_bar_row: u16,
}

impl NavigationState {
    pub fn new() -> Self {
        let tabs = vec![
            Tab { label: "Dashboard".into(), id: "dashboard".into(), unread: 0 },
            Tab { label: "Communities".into(), id: "communities".into(), unread: 0 },
            Tab { label: "DMs".into(), id: "dms".into(), unread: 0 },
            Tab { label: "Friends".into(), id: "friends".into(), unread: 0 },
        ];
        Self {
            view_stack: vec![ViewKind::Dashboard],
            input_mode: false,
            input_context: InputContext::Compose,
            overlay: None,
            confirm: ConfirmState::new(),
            emoji_picker: EmojiPickerState::new(),
            search: None,
            file_search: None,
            sidebar_visible: true,
            tab_bar: TabBarState::new(tabs),
            tab_unreads: HashMap::new(),
            focus_ring: FocusRing::new(vec![
                FocusId::DashIdentity,
                FocusId::DashNode,
                FocusId::ChannelTree,
                FocusId::FriendList,
            ]),
            terminal_focused: true,
            tab_bar_row: 0,
        }
    }

    pub fn current_view(&self) -> &ViewKind {
        self.view_stack.last().expect("view stack is never empty")
    }

    /// Push a view. Dashboard clears the stack first.
    pub fn push_view(&mut self, view: ViewKind) {
        if matches!(view, ViewKind::Dashboard) {
            self.view_stack.clear();
            self.view_stack.push(ViewKind::Dashboard);
        } else if self.view_stack.last() != Some(&view) {
            self.view_stack.push(view);
        }
        self.input_mode = false;
        self.input_context = InputContext::Compose;
    }

    /// Pop the current view. No-op if on Dashboard.
    pub fn pop_view(&mut self) {
        if self.view_stack.len() > 1 {
            self.view_stack.pop();
            self.input_mode = false;
        }
    }

    pub fn sync_tab_to_view(&mut self, view: &ViewKind) {
        self.tab_bar.sync_to_view(view);
    }

    pub fn input_mode_allowed(&self) -> bool {
        matches!(
            self.current_view(),
            ViewKind::ChannelWatch { .. } | ViewKind::DmInbox | ViewKind::DmThread { .. }
        )
    }
}

impl Default for NavigationState {
    fn default() -> Self {
        Self::new()
    }
}
