//! Channel tree — sidebar navigation for communities and channels.
//!
//! Renders a collapsible tree: Community → Category → Channel, plus a
//! DM section at the bottom. Each node shows unread count. Tree connectors
//! use Unicode box-drawing (├──, └──, │) with ASCII fallback for TERM=dumb.
//!
//! Navigation:
//! - `j/k` or arrows move selection
//! - `l` or `Enter` expands/enters a node
//! - `h` collapses or moves to parent
//!
//! Source patterns:
//! - oxicord `presentation/widgets/guilds_tree.rs` — TreeNodeId, flatten, connectors
//! - siggy `ui/sidebar.rs` — responsive auto-hide, unread counts

use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState};
use ratatui::Frame;

use super::Component;
use crate::helpers;
use crate::tui::action::Action;

/// Unique identifier for a tree node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TreeNodeId {
    /// A community header (expandable).
    Community(String),
    /// A category within a community (expandable).
    Category { community: String, category: String },
    /// A channel within a community (selectable).
    Channel { community: String, channel: String },
    /// The DMs section header (expandable).
    DirectMessages,
    /// A DM conversation (selectable).
    DmUser(String),
}

/// A single node in the tree.
#[derive(Debug, Clone)]
pub struct TreeNode {
    /// Node identity.
    pub id: TreeNodeId,
    /// Display label.
    pub label: String,
    /// Nesting depth (0 = root).
    pub depth: u16,
    /// Whether this node has children.
    pub has_children: bool,
    /// Unread message count (0 = no badge).
    pub unread: u32,
    /// Channel kind indicator (e.g., "text", "voice"). Empty for non-channels.
    pub kind: String,
}

/// Channel tree component state.
pub struct ChannelTree {
    /// All tree nodes in display order (pre-flattened).
    nodes: Vec<TreeNode>,
    /// Set of expanded node IDs.
    expanded: HashSet<TreeNodeId>,
    /// Ratatui list selection state.
    list_state: ListState,
    /// Whether this component is focused.
    is_focused: bool,
    /// Whether Unicode connectors are available.
    use_unicode: bool,
}

impl ChannelTree {
    /// Create a new channel tree.
    pub fn new(use_unicode: bool) -> Self {
        Self {
            nodes: Vec::new(),
            expanded: HashSet::new(),
            list_state: ListState::default(),
            is_focused: false,
            use_unicode,
        }
    }

    /// Set the tree nodes from community/channel data.
    ///
    /// Builds the flat node list from community memberships. Communities
    /// are sorted alphabetically. Channels within each community are
    /// sorted by category then sort_order.
    /// Set a single channel's unread count and recompute the community aggregate.
    pub fn set_channel_unread(&mut self, community: &str, channel: &str, count: u32) {
        if let Some(node) = self.nodes.iter_mut().find(|n| matches!(&n.id, TreeNodeId::Channel { community: c, channel: ch } if c == community && ch == channel)) {
            node.unread = count;
        }
        // Recompute community aggregate
        let total: u32 = self.nodes.iter()
            .filter_map(|n| match &n.id {
                TreeNodeId::Channel { community: c, .. } if c == community => Some(n.unread),
                _ => None,
            })
            .sum();
        if let Some(node) = self.nodes.iter_mut().find(|n| matches!(&n.id, TreeNodeId::Community(k) if k == community)) {
            node.unread = total;
        }
    }

    /// Update unread counts in bulk without rebuilding the tree.
    #[allow(dead_code)] // Used by set_communities diff path; direct callers arrive in M1
    pub fn update_unreads(&mut self, unreads: &std::collections::HashMap<(String, String), u32>) {
        for node in &mut self.nodes {
            if let TreeNodeId::Channel { ref community, ref channel } = node.id {
                if let Some(&count) = unreads.get(&(community.clone(), channel.clone())) {
                    node.unread = count;
                }
            }
        }
        // Recompute community aggregates
        let totals: std::collections::HashMap<String, u32> = self.nodes.iter()
            .filter_map(|n| match &n.id {
                TreeNodeId::Channel { community, .. } => Some((community.clone(), n.unread)),
                _ => None,
            })
            .fold(std::collections::HashMap::new(), |mut acc, (k, v)| {
                *acc.entry(k).or_default() += v;
                acc
            });
        for node in &mut self.nodes {
            if let TreeNodeId::Community(ref key) = node.id {
                if let Some(&total) = totals.get(key) {
                    node.unread = total;
                }
            }
        }
    }

    pub fn set_communities(
        &mut self,
        communities: &[(String, String, Vec<ChannelEntry>)], // (gov_key, name, channels)
        dm_peers: &[(String, String, u32)], // (peer_key, display_name, unread)
    ) {
        // Skip full rebuild if only community keys match — just update unreads
        let new_keys: Vec<&str> = communities.iter().map(|(k, _, _)| k.as_str()).collect();
        let existing_keys: Vec<&str> = self.nodes.iter()
            .filter_map(|n| match &n.id {
                TreeNodeId::Community(k) => Some(k.as_str()),
                _ => None,
            })
            .collect();
        if new_keys == existing_keys && !new_keys.is_empty() {
            // Keys unchanged — update unreads in-place
            for (gov_key, _, channels) in communities {
                for ch in channels {
                    if let Some(node) = self.nodes.iter_mut().find(|n| matches!(&n.id, TreeNodeId::Channel { community, channel } if community == gov_key && channel == &ch.id)) {
                        node.unread = ch.unread;
                    }
                }
                if let Some(node) = self.nodes.iter_mut().find(|n| matches!(&n.id, TreeNodeId::Community(k) if k == gov_key)) {
                    node.unread = channels.iter().map(|c| c.unread).sum();
                }
            }
            return;
        }

        self.nodes.clear();

        for (gov_key, name, channels) in communities {
            // Community header
            let community_id = TreeNodeId::Community(gov_key.clone());
            self.nodes.push(TreeNode {
                id: community_id.clone(),
                label: name.clone(),
                depth: 0,
                has_children: !channels.is_empty(),
                unread: channels.iter().map(|c| c.unread).sum(),
                kind: String::new(),
            });

            if self.expanded.contains(&community_id) {
                // Sort channels by category then sort_order for consistent display
                let mut sorted_channels = channels.clone();
                sorted_channels.sort_by(|a, b| {
                    a.category.cmp(&b.category).then(a.sort_order.cmp(&b.sort_order))
                });

                // Group channels by category
                let mut current_category: Option<String> = None;
                for ch in &sorted_channels {
                    let cat = ch.category.as_deref().unwrap_or("");
                    if !cat.is_empty() && current_category.as_deref() != Some(cat) {
                        current_category = Some(cat.to_string());
                        let cat_id = TreeNodeId::Category {
                            community: gov_key.clone(),
                            category: cat.to_string(),
                        };
                        self.nodes.push(TreeNode {
                            id: cat_id,
                            label: cat.to_string(),
                            depth: 1,
                            has_children: true,
                            unread: 0,
                            kind: String::new(),
                        });
                    }

                    let depth = if cat.is_empty() { 1 } else { 2 };
                    self.nodes.push(TreeNode {
                        id: TreeNodeId::Channel {
                            community: gov_key.clone(),
                            channel: ch.id.clone(),
                        },
                        label: ch.name.clone(),
                        depth,
                        has_children: false,
                        unread: ch.unread,
                        kind: ch.kind.clone(),
                    });
                }
            }
        }

        // DM section
        if !dm_peers.is_empty() {
            let dm_id = TreeNodeId::DirectMessages;
            let dm_unread: u32 = dm_peers.iter().map(|(_, _, u)| u).sum();
            self.nodes.push(TreeNode {
                id: dm_id.clone(),
                label: "Direct Messages".into(),
                depth: 0,
                has_children: true,
                unread: dm_unread,
                kind: String::new(),
            });

            if self.expanded.contains(&dm_id) {
                for (peer_key, name, unread) in dm_peers {
                    self.nodes.push(TreeNode {
                        id: TreeNodeId::DmUser(peer_key.clone()),
                        label: name.clone(),
                        depth: 1,
                        has_children: false,
                        unread: *unread,
                        kind: String::new(),
                    });
                }
            }
        }

        // Select first node if nothing selected
        if self.list_state.selected().is_none() && !self.nodes.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    /// Get the currently selected node ID.
    pub fn selected_id(&self) -> Option<&TreeNodeId> {
        self.list_state
            .selected()
            .and_then(|i| self.nodes.get(i))
            .map(|n| &n.id)
    }

    /// Toggle expand/collapse of the selected node.
    fn toggle_expand(&mut self) {
        let Some(idx) = self.list_state.selected() else { return };
        let Some(node) = self.nodes.get(idx) else { return };
        if !node.has_children {
            return;
        }
        let id = node.id.clone();
        if self.expanded.contains(&id) {
            self.expanded.remove(&id);
        } else {
            self.expanded.insert(id);
        }
    }

    /// Collapse the selected node, or move to its parent.
    fn collapse_or_parent(&mut self) {
        let Some(idx) = self.list_state.selected() else { return };
        let Some(node) = self.nodes.get(idx) else { return };

        // If expanded, collapse
        if node.has_children && self.expanded.contains(&node.id) {
            self.expanded.remove(&node.id);
            return;
        }

        // Otherwise move to parent (first node with lower depth above us)
        let target_depth = node.depth.saturating_sub(1);
        for i in (0..idx).rev() {
            if self.nodes[i].depth <= target_depth {
                self.list_state.select(Some(i));
                return;
            }
        }
    }

    /// Build tree connectors for a node at the given index.
    ///
    /// Uses Unicode box-drawing characters on capable terminals:
    ///   ├── for nodes with siblings below
    ///   └── for the last node in a group
    ///   │   for continuation lines
    ///
    /// Falls back to ASCII on TERM=dumb / non-Unicode locales:
    ///   |-- for nodes with siblings below
    ///   `-- for the last node in a group
    ///   |   for continuation lines
    fn connector_for(&self, index: usize) -> &str {
        let node = &self.nodes[index];
        if node.depth == 0 {
            return "";
        }

        // Check if this is the last node at this depth in its parent group
        let is_last = self.nodes[index + 1..]
            .iter()
            .take_while(|n| n.depth >= node.depth)
            .all(|n| n.depth > node.depth);

        if is_last {
            if self.use_unicode { "└── " } else { "`-- " }
        } else if self.use_unicode { "├── " } else { "|-- " }
    }

    /// Build list items for rendering.
    fn build_items(&self) -> Vec<ListItem<'static>> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, node)| {
                // Depth-based indent: use continuation lines (│ or |) for
                // each ancestor level, then the connector at the node's level.
                let pipe = if self.use_unicode { "│   " } else { "|   " };
                let indent = pipe.repeat(node.depth.saturating_sub(1) as usize);
                let connector = self.connector_for(i);

                let expand_marker = if node.has_children {
                    if self.expanded.contains(&node.id) {
                        if self.use_unicode { "▾ " } else { "v " }
                    } else if self.use_unicode { "▸ " } else { "> " }
                } else {
                    "  "
                };

                let channel_prefix = if node.kind.is_empty() {
                    ""
                } else {
                    match node.kind.as_str() {
                        "voice" => if self.use_unicode { "🔊 " } else { "[V] " },
                        "announcement" => if self.use_unicode { "📢 " } else { "[A] " },
                        "forum" => if self.use_unicode { "📋 " } else { "[F] " },
                        "stage" => if self.use_unicode { "🎙 " } else { "[S] " },
                        "media" => if self.use_unicode { "🖼 " } else { "[M] " },
                        "events" => if self.use_unicode { "📅 " } else { "[E] " },
                        _ => "# ",
                    }
                };

                let label = helpers::sanitize_for_display(&node.label);

                let unread_badge = super::unread_badge::format_unread(node.unread);

                let mut spans = vec![
                    Span::raw(format!("{indent}{connector}{expand_marker}{channel_prefix}")),
                    Span::raw(label),
                ];

                if !unread_badge.is_empty() {
                    spans.push(Span::styled(format!(" {unread_badge}"), Style::new().bold()));
                }

                ListItem::new(Line::from(spans))
            })
            .collect()
    }
}

/// Channel data for tree construction.
#[derive(Debug, Clone)]
pub struct ChannelEntry {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub category: Option<String>,
    pub unread: u32,
    pub sort_order: u16,
}

impl Component for ChannelTree {
    fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                let max = self.nodes.len().saturating_sub(1);
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1).min(max)));
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some(i.saturating_sub(1)));
                None
            }
            KeyCode::Char('l') | KeyCode::Enter | KeyCode::Right => {
                let selected = self.selected_id().cloned();
                match selected {
                    Some(TreeNodeId::Channel { community, channel }) => {
                        Some(Action::ShowChannel { community, channel })
                    }
                    Some(TreeNodeId::DmUser(peer_key)) => {
                        Some(Action::ShowDmThread { peer_key })
                    }
                    Some(id) if self.nodes.iter().any(|n| n.id == id && n.has_children) => {
                        self.toggle_expand();
                        None
                    }
                    _ => None,
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.collapse_or_parent();
                None
            }
            KeyCode::Home => {
                if !self.nodes.is_empty() {
                    self.list_state.select(Some(0));
                }
                None
            }
            KeyCode::End => {
                if !self.nodes.is_empty() {
                    self.list_state.select(Some(self.nodes.len() - 1));
                }
                None
            }
            _ => None,
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect) -> anyhow::Result<()> {
        let items = self.build_items();

        let block = Block::bordered()
            .title(" Channels ")
            .border_style(if self.is_focused {
                Style::new()
            } else {
                Style::new().dim()
            });

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::new().reversed());

        frame.render_stateful_widget(list, area, &mut self.list_state);
        Ok(())
    }

    fn set_focused(&mut self, focused: bool) {
        self.is_focused = focused;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_channels() -> Vec<ChannelEntry> {
        vec![
            ChannelEntry {
                id: "ch-general".into(),
                name: "general".into(),
                kind: "text".into(),
                category: None,
                unread: 3,
                sort_order: 0,
            },
            ChannelEntry {
                id: "ch-voice".into(),
                name: "voice-1".into(),
                kind: "voice".into(),
                category: Some("Voice".into()),
                unread: 0,
                sort_order: 1,
            },
        ]
    }

    #[test]
    fn set_communities_builds_nodes() {
        let mut tree = ChannelTree::new(true);
        tree.set_communities(
            &[("gov1".into(), "dev-team".into(), test_channels())],
            &[],
        );
        // Should have just the community header (collapsed by default)
        assert_eq!(tree.nodes.len(), 1);
        assert_eq!(tree.nodes[0].label, "dev-team");
        assert_eq!(tree.nodes[0].unread, 3);
    }

    #[test]
    fn expand_shows_children() {
        let mut tree = ChannelTree::new(true);
        tree.expanded.insert(TreeNodeId::Community("gov1".into()));
        tree.set_communities(
            &[("gov1".into(), "dev-team".into(), test_channels())],
            &[],
        );
        // Community header + 2 channels + 1 category = 4 nodes
        assert!(tree.nodes.len() > 1);
    }

    #[test]
    fn dm_section_shows_peers() {
        let mut tree = ChannelTree::new(true);
        tree.expanded.insert(TreeNodeId::DirectMessages);
        tree.set_communities(
            &[],
            &[("pk1".into(), "alice".into(), 2), ("pk2".into(), "bob".into(), 0)],
        );
        // DM header + 2 peers = 3 nodes
        assert_eq!(tree.nodes.len(), 3);
    }

    #[test]
    fn selected_id_returns_current() {
        let mut tree = ChannelTree::new(true);
        tree.set_communities(
            &[("gov1".into(), "dev-team".into(), Vec::new())],
            &[],
        );
        assert_eq!(
            tree.selected_id(),
            Some(&TreeNodeId::Community("gov1".into()))
        );
    }

    #[test]
    fn toggle_expand_collapse() {
        let mut tree = ChannelTree::new(true);
        tree.set_communities(
            &[("gov1".into(), "dev-team".into(), test_channels())],
            &[],
        );
        // Initially collapsed
        assert!(!tree.expanded.contains(&TreeNodeId::Community("gov1".into())));
        tree.list_state.select(Some(0));
        tree.toggle_expand();
        assert!(tree.expanded.contains(&TreeNodeId::Community("gov1".into())));
        tree.toggle_expand();
        assert!(!tree.expanded.contains(&TreeNodeId::Community("gov1".into())));
    }

    #[test]
    fn collapse_or_parent_moves_up() {
        let mut tree = ChannelTree::new(true);
        tree.expanded.insert(TreeNodeId::Community("gov1".into()));
        tree.set_communities(
            &[("gov1".into(), "dev-team".into(), test_channels())],
            &[],
        );
        // Select a child channel
        tree.list_state.select(Some(1));
        tree.collapse_or_parent();
        // Should have moved to parent (community header at index 0)
        assert_eq!(tree.list_state.selected(), Some(0));
    }
}
