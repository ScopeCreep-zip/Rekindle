//! Channel tree data types.

/// Unique identifier for a tree node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TreeNodeId {
    Community(String),
    Category { community: String, category: String },
    Channel { community: String, channel: String },
    DirectMessages,
    DmUser(String),
}

/// A single node in the flattened tree.
#[derive(Debug, Clone)]
pub struct TreeNode {
    pub id: TreeNodeId,
    pub label: String,
    pub depth: u16,
    pub has_children: bool,
    pub unread: u32,
    pub kind: String,
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
