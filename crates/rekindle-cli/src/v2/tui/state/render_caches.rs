//! Render caches — ListState, cached ListItems, click targets.
//!
//! Separate from TuiState to allow `&mut RenderCaches` during draw
//! while input handlers hold `&mut RenderCaches` for search list and click targets.

use std::collections::HashMap;

use ratatui::layout::Rect;
use ratatui::widgets::{ListItem, ListState};

use crate::v2::tui::focus::FocusId;
use super::channels::ChannelKey;
use super::navigation::ViewKindTag;

/// Cached render state for a message list. Only rebuilds ListItems
/// when `generation` diverges from the source's generation counter.
///
/// `items` and `generation` are private — the only way to update them
/// is through `rebuild()`, which sets both atomically. This prevents
/// generation/items desync where items are updated but generation is not.
#[derive(Debug)]
pub struct MessageRenderCache {
    items: Vec<ListItem<'static>>,
    /// Must remain pub — `frame.render_stateful_widget` takes `&mut ListState`.
    pub list_state: ListState,
    generation: u64,
}

impl MessageRenderCache {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            list_state: ListState::default(),
            generation: u64::MAX,
        }
    }

    /// Cached ListItems for rendering. Read-only — mutation goes through `rebuild`.
    pub fn items(&self) -> &[ListItem<'static>] {
        &self.items
    }

    /// Returns true if the user is at (or past) the last item — meaning
    /// new messages should auto-scroll to keep the latest visible.
    pub fn is_at_bottom(&self) -> bool {
        match self.list_state.selected() {
            None => true, // no selection = treat as "follow latest"
            Some(idx) => self.items.is_empty() || idx >= self.items.len().saturating_sub(1),
        }
    }

    #[must_use]
    pub fn needs_rebuild(&self, source_generation: u64) -> bool {
        self.generation != source_generation
    }

    /// Atomically replace items and update generation. The only way to
    /// set items or generation. Prevents desync.
    pub fn rebuild(&mut self, items: Vec<ListItem<'static>>, source_generation: u64) {
        self.items = items;
        self.generation = source_generation;
    }
}

impl Default for MessageRenderCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Cached file content for file preview. Read from disk once on miss.
#[derive(Clone, Debug)]
pub struct FilePreviewCache {
    pub path: String,
    pub content: String,
    pub line_count: usize,
}

/// All render caches. Sibling to TuiState in the machine loop.
#[derive(Debug, Default)]
pub struct RenderCaches {
    /// Keyed by peer_key.
    pub dm_thread: HashMap<String, MessageRenderCache>,
    pub channel: HashMap<ChannelKey, MessageRenderCache>,
    /// Keyed by peer_key. Right side of channel watch.
    pub split_dm: HashMap<String, MessageRenderCache>,
    /// Keyed by `ViewKindTag` (pure discriminant, no String fields).
    pub click_targets: HashMap<ViewKindTag, HashMap<FocusId, Rect>>,
    /// Synced from `SessionState.dm_selected_peer` at render time.
    /// Not authoritative — `SessionState` is the source of truth.
    pub dm_inbox_list: ListState,
    /// Synced from `SessionState.friend_selected_key` at render time.
    /// Not authoritative — `SessionState` is the source of truth.
    pub friend_list: ListState,
    pub file_preview: Option<FilePreviewCache>,
    pub doctor_list: ListState,
    pub moderation_list: ListState,
    pub invite_list: ListState,
    pub events_list: ListState,
    /// Tab bar click regions — populated by tab_bar::render, read by input mouse handler.
    pub tab_bar_click_regions: Vec<(u16, u16, usize)>,
    /// Search overlay list selection — mutable widget state for search results.
    pub search_list: ListState,
    /// Cached route health gradient meter for the dashboard Node panel.
    pub route_health_meter: Option<crate::v2::tui::widgets::meter::CachedMeter>,
}

impl RenderCaches {
    pub fn new() -> Self {
        Self::default()
    }
}
