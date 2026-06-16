//! Tab bar state — selection, unread counts, click regions, scroll offset.

use super::navigation::ViewKind;

/// One tab entry.
#[derive(Clone, Debug)]
pub struct Tab {
    pub label: String,
    pub id: String,
    pub unread: u32,
}

/// Tab bar selection, scroll, click-region state.
#[derive(Debug)]
pub struct TabBarState {
    tabs: Vec<Tab>,
    selected: usize,
    scroll_offset: usize,
}

impl TabBarState {
    pub fn new(tabs: Vec<Tab>) -> Self {
        Self { tabs, selected: 0, scroll_offset: 0 }
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub fn next(&mut self) {
        if !self.tabs.is_empty() {
            self.selected = (self.selected + 1) % self.tabs.len();
            self.ensure_visible();
        }
    }

    pub fn prev(&mut self) {
        if !self.tabs.is_empty() {
            self.selected = if self.selected == 0 { self.tabs.len() - 1 } else { self.selected - 1 };
            self.ensure_visible();
        }
    }

    pub fn select(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.selected = index;
            self.ensure_visible();
        }
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.tabs.get(self.selected).map(|t| t.id.as_str())
    }

    pub fn select_by_id(&mut self, id: &str) {
        if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
            self.selected = i;
            self.ensure_visible();
        }
    }

    /// Sync tab selection to match a view kind.
    pub fn sync_to_view(&mut self, view: &ViewKind) {
        let tab_id = match view {
            ViewKind::Dashboard | ViewKind::Doctor
            | ViewKind::IdentitySettings | ViewKind::FilePreview { .. } => "dashboard",
            ViewKind::DmInbox | ViewKind::DmThread { .. } => "dms",
            ViewKind::FriendList => "friends",
            ViewKind::ChannelWatch { .. } | ViewKind::CommunityInfo { .. }
            | ViewKind::VoiceSession { .. } | ViewKind::Moderation { .. }
            | ViewKind::Invite { .. } | ViewKind::Events { .. }
            | ViewKind::Onboarding { .. } => "communities",
        };
        self.select_by_id(tab_id);
    }

    pub fn increment_unread(&mut self, tab_id: &str) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.unread = tab.unread.saturating_add(1);
        }
    }

    pub fn clear_unread(&mut self, tab_id: &str) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.unread = 0;
        }
    }

    fn ensure_visible(&mut self) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        }
    }
}
