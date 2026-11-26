//! The served `disabledBy` of each interactive node (plan E7.7 CR1).

use rekindle_types::dto::skin_tree::{DisabledBy, SkinNode};

use super::golden_tests::main_layout;

fn find<'a>(nodes: &'a [SkinNode], id: &str) -> Option<&'a SkinNode> {
    nodes.iter().find_map(|n| {
        if n.id == id {
            Some(n)
        } else {
            find(&n.children, id)
        }
    })
}

fn each(nodes: &[SkinNode], f: &mut impl FnMut(&SkinNode)) {
    for n in nodes {
        f(n);
        each(&n.children, f);
    }
}

#[test]
fn xslimfire_chat_button_is_disabled_by_its_packs_art() {
    let tree = main_layout("xslimfire").tree.unwrap();
    let chat = find(&tree.root, "ChatButton").unwrap();
    assert_eq!(chat.disabled_by, Some(DisabledBy::Pack));
    assert_eq!(serde_json::to_value(chat).unwrap()["disabledBy"], "pack");
}

#[test]
fn the_composed_communities_tab_is_disabled_by_the_shell() {
    let tree = main_layout("xslimfire").tree.unwrap();
    let tab = find(&tree.root, "TabDownloadButton.Communities").unwrap();
    assert_eq!(tab.disabled_by, Some(DisabledBy::Shell));
    assert_eq!(serde_json::to_value(tab).unwrap()["disabledBy"], "shell");
}

#[test]
fn every_interactive_node_and_only_those_say_who_disables_it() {
    for id in ["xslimfire", "combat"] {
        let tree = main_layout(id).tree.unwrap();
        each(&tree.root, &mut |n| {
            assert_eq!(n.disabled_by.is_some(), n.interactive(), "{id} {}", n.id);
        });
    }
}
