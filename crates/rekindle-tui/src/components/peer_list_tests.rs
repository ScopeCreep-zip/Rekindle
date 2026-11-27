use super::*;

fn entry(key: &str, name: &str, status: &str, group: MemberGroup) -> PeerEntry {
    PeerEntry {
        key: key.into(),
        display_name: name.into(),
        status: status.into(),
        role: None,
        group,
    }
}

/// The Host's order: the admin group, then Online, then Offline. Bob is
/// offline yet the Host put him under Online, so the list must show
/// him there: it renders the Host's groups, it does not regroup.
fn test_members() -> Vec<PeerEntry> {
    let admins = MemberGroup::Role {
        id: "02".repeat(16),
        name: "Admins".into(),
    };
    vec![
        entry("pk1", "alice", "online", admins),
        entry("pk3", "carol", "online", MemberGroup::Online),
        entry("pk2", "bob", "offline", MemberGroup::Online),
        entry("pk4", "dave", "away", MemberGroup::Offline),
    ]
}

fn text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
fn renders_host_groups_in_host_order() {
    let mut list = PeerList::new(true);
    list.set_members(test_members());
    let lines: Vec<String> = list.build_lines().iter().map(text).collect();
    assert_eq!(lines[0], " Admins (1)");
    assert!(lines[1].contains("alice"));
    assert_eq!(lines[2], " Online (2)");
    assert!(lines[3].contains("carol"));
    assert!(lines[4].contains("bob"));
    assert_eq!(lines[5], " Offline (1)");
    assert!(lines[6].contains("dave"));
    assert_eq!(lines.len(), 7);
}

#[test]
fn len_and_is_empty() {
    let mut list = PeerList::new(true);
    assert!(list.is_empty());
    assert_eq!(list.len(), 0);
    list.set_members(test_members());
    assert!(!list.is_empty());
    assert_eq!(list.len(), 4);
}

#[test]
fn presence_indicator_unicode() {
    let (glyph, label) = presence_indicator("online", true);
    assert_eq!(glyph, "●");
    assert_eq!(label, "[ONLINE]");
}

#[test]
fn presence_indicator_ascii() {
    let (glyph, label) = presence_indicator("online", false);
    assert_eq!(glyph, "o");
    assert_eq!(label, "[ONLINE]");
}
