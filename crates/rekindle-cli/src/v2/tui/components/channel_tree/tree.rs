//! Channel tree state — community/channel hierarchy with expand/collapse.

use std::collections::HashSet;

use ratatui::widgets::ListState;

use super::types::{ChannelEntry, TreeNode, TreeNodeId};

/// Channel tree component state.
pub struct ChannelTree {
    pub(super) nodes: Vec<TreeNode>,
    pub(super) expanded: HashSet<TreeNodeId>,
    pub(super) list_state: ListState,
    pub(super) is_focused: bool,
    pub(super) use_unicode: bool,
}

impl ChannelTree {
    pub fn new(use_unicode: bool) -> Self {
        Self {
            nodes: Vec::new(),
            expanded: HashSet::new(),
            list_state: ListState::default(),
            is_focused: false,
            use_unicode,
        }
    }

    /// Set a single channel's unread count and recompute community aggregate.
    pub fn set_channel_unread(&mut self, community: &str, channel: &str, count: u32) {
        if let Some(node) = self.nodes.iter_mut().find(|n| {
            matches!(&n.id, TreeNodeId::Channel { community: c, channel: ch } if c == community && ch == channel)
        }) {
            node.unread = count;
        }
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

    /// Build the tree from community/channel data.
    pub fn set_communities(
        &mut self,
        communities: &[(String, String, Vec<ChannelEntry>)],
        dm_peers: &[(String, String, u32)],
    ) {
        // Skip full rebuild if community keys unchanged — just update unreads
        let new_keys: Vec<&str> = communities.iter().map(|(k, _, _)| k.as_str()).collect();
        let existing_keys: Vec<&str> = self.nodes.iter()
            .filter_map(|n| match &n.id { TreeNodeId::Community(k) => Some(k.as_str()), _ => None })
            .collect();

        if new_keys == existing_keys && !new_keys.is_empty() {
            for (gov_key, _, channels) in communities {
                for ch in channels {
                    if let Some(node) = self.nodes.iter_mut().find(|n| {
                        matches!(&n.id, TreeNodeId::Channel { community, channel } if community == gov_key && channel == &ch.id)
                    }) {
                        node.unread = ch.unread;
                    }
                }
            }
            return;
        }

        self.nodes.clear();

        for (gov_key, name, channels) in communities {
            let community_id = TreeNodeId::Community(gov_key.clone());
            self.nodes.push(TreeNode {
                id: community_id.clone(), label: name.clone(), depth: 0,
                has_children: !channels.is_empty(),
                unread: channels.iter().map(|c| c.unread).sum(),
                kind: String::new(),
            });

            if self.expanded.contains(&community_id) {
                let mut sorted = channels.clone();
                sorted.sort_by(|a, b| a.category.cmp(&b.category).then(a.sort_order.cmp(&b.sort_order)));

                let mut current_category: Option<String> = None;
                for ch in &sorted {
                    let cat = ch.category.as_deref().unwrap_or("");
                    if !cat.is_empty() && current_category.as_deref() != Some(cat) {
                        current_category = Some(cat.to_string());
                        self.nodes.push(TreeNode {
                            id: TreeNodeId::Category { community: gov_key.clone(), category: cat.to_string() },
                            label: cat.to_string(), depth: 1, has_children: true, unread: 0, kind: String::new(),
                        });
                    }
                    let depth = if cat.is_empty() { 1 } else { 2 };
                    self.nodes.push(TreeNode {
                        id: TreeNodeId::Channel { community: gov_key.clone(), channel: ch.id.clone() },
                        label: ch.name.clone(), depth, has_children: false, unread: ch.unread, kind: ch.kind.clone(),
                    });
                }
            }
        }

        if !dm_peers.is_empty() {
            let dm_id = TreeNodeId::DirectMessages;
            self.nodes.push(TreeNode {
                id: dm_id.clone(), label: "Direct Messages".into(), depth: 0,
                has_children: true, unread: dm_peers.iter().map(|(_, _, u)| u).sum(),
                kind: String::new(),
            });
            if self.expanded.contains(&dm_id) {
                for (peer_key, name, unread) in dm_peers {
                    self.nodes.push(TreeNode {
                        id: TreeNodeId::DmUser(peer_key.clone()),
                        label: name.clone(), depth: 1, has_children: false, unread: *unread, kind: String::new(),
                    });
                }
            }
        }

        if self.list_state.selected().is_none() && !self.nodes.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    pub fn selected_id(&self) -> Option<&TreeNodeId> {
        self.list_state.selected().and_then(|i| self.nodes.get(i)).map(|n| &n.id)
    }

    pub fn toggle_expand(&mut self) {
        let Some(idx) = self.list_state.selected() else { return };
        let Some(node) = self.nodes.get(idx) else { return };
        if !node.has_children { return; }
        let id = node.id.clone();
        if self.expanded.contains(&id) { self.expanded.remove(&id); }
        else { self.expanded.insert(id); }
    }

    pub fn expand(&mut self, id: &TreeNodeId) { self.expanded.insert(id.clone()); }

    pub fn collapse_or_parent(&mut self) {
        let Some(idx) = self.list_state.selected() else { return };
        let Some(node) = self.nodes.get(idx) else { return };

        if node.has_children && self.expanded.contains(&node.id) {
            self.expanded.remove(&node.id);
            return;
        }

        let target_depth = node.depth.saturating_sub(1);
        for i in (0..idx).rev() {
            if self.nodes[i].depth <= target_depth {
                self.list_state.select(Some(i));
                return;
            }
        }
    }
}
