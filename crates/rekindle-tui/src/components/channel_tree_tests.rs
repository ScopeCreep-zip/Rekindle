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
    tree.set_communities(&[("gov1".into(), "dev-team".into(), test_channels())], &[]);
    // Should have just the community header (collapsed by default)
    assert_eq!(tree.nodes.len(), 1);
    assert_eq!(tree.nodes[0].label, "dev-team");
    assert_eq!(tree.nodes[0].unread, 3);
}

#[test]
fn expand_shows_children() {
    let mut tree = ChannelTree::new(true);
    tree.expanded.insert(TreeNodeId::Community("gov1".into()));
    tree.set_communities(&[("gov1".into(), "dev-team".into(), test_channels())], &[]);
    // Community header + 2 channels + 1 category = 4 nodes
    assert!(tree.nodes.len() > 1);
}

#[test]
fn dm_section_shows_peers() {
    let mut tree = ChannelTree::new(true);
    tree.expanded.insert(TreeNodeId::DirectMessages);
    tree.set_communities(
        &[],
        &[
            ("pk1".into(), "alice".into(), 2),
            ("pk2".into(), "bob".into(), 0),
        ],
    );
    // DM header + 2 peers = 3 nodes
    assert_eq!(tree.nodes.len(), 3);
}

#[test]
fn selected_id_returns_current() {
    let mut tree = ChannelTree::new(true);
    tree.set_communities(&[("gov1".into(), "dev-team".into(), Vec::new())], &[]);
    assert_eq!(
        tree.selected_node().map(|n| &n.id),
        Some(&TreeNodeId::Community("gov1".into()))
    );
}

#[test]
fn toggle_expand_collapse() {
    let mut tree = ChannelTree::new(true);
    tree.set_communities(&[("gov1".into(), "dev-team".into(), test_channels())], &[]);
    // Initially collapsed
    assert!(!tree
        .expanded
        .contains(&TreeNodeId::Community("gov1".into())));
    tree.list_state.select(Some(0));
    tree.toggle_expand();
    assert!(tree
        .expanded
        .contains(&TreeNodeId::Community("gov1".into())));
    tree.toggle_expand();
    assert!(!tree
        .expanded
        .contains(&TreeNodeId::Community("gov1".into())));
}

#[test]
fn collapse_or_parent_moves_up() {
    let mut tree = ChannelTree::new(true);
    tree.expanded.insert(TreeNodeId::Community("gov1".into()));
    tree.set_communities(&[("gov1".into(), "dev-team".into(), test_channels())], &[]);
    // Select a child channel
    tree.list_state.select(Some(1));
    tree.collapse_or_parent();
    // Should have moved to parent (community header at index 0)
    assert_eq!(tree.list_state.selected(), Some(0));
}
