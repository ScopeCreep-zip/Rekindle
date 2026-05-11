//! Tab bar state — selection, unread counts, click regions.

/// A tab entry.
#[derive(Debug, Clone)]
pub struct Tab {
    pub label: String,
    pub id: String,
    pub unread: u32,
}

/// Tab bar state.
pub struct TabBarState {
    pub tabs: Vec<Tab>,
    pub selected: usize,
    scroll_offset: usize,
    pub click_regions: Vec<(u16, u16, usize)>,
}

impl TabBarState {
    pub fn new(tabs: Vec<Tab>) -> Self {
        Self { tabs, selected: 0, scroll_offset: 0, click_regions: Vec::new() }
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

    pub fn sync_to_view(&mut self, view: &crate::v2::views::ViewKind) {
        use crate::v2::views::ViewKind;
        let tab_id = match view {
            ViewKind::Dashboard | ViewKind::Doctor | ViewKind::IdentitySettings | ViewKind::FilePreview { .. } => "dashboard",
            ViewKind::DmInbox | ViewKind::DmThread { .. } => "dms",
            ViewKind::FriendList => "friends",
            ViewKind::ChannelWatch { .. } | ViewKind::CommunityInfo { .. } | ViewKind::VoiceSession { .. } => "communities",
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

    pub fn click_tab(&self, column: u16, row: u16, tab_row: u16) -> Option<usize> {
        if row != tab_row { return None; }
        self.click_regions.iter()
            .find(|&&(start_x, end_x, _)| column >= start_x && column < end_x)
            .map(|&(_, _, idx)| idx)
    }

    fn ensure_visible(&mut self) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        }
    }

    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }
}
