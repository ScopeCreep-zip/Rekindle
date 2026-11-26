use super::build_roster_entries;

#[test]
fn roster_includes_self_with_empty_transport() {
    // The decoupled-roster fix hinges on this: a present member's
    // broadcast must carry itself, even before any peer is in its
    // transport, or a later joiner never learns about it.
    let entries = build_roster_entries(
        "me",
        Some(vec![1, 2, 3]),
        Some("Me Name".to_string()),
        vec![],
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].pseudonym_key, "me");
    assert_eq!(entries[0].route_blob, vec![1, 2, 3]);
    assert_eq!(entries[0].display_name.as_deref(), Some("Me Name"));
}

#[test]
fn roster_appends_self_after_peers() {
    let peers = vec![("peerA".to_string(), vec![9], Some("Peer A".to_string()))];
    let entries = build_roster_entries("me", Some(vec![1]), None, peers);
    assert_eq!(entries.len(), 2);
    let peer_a = entries
        .iter()
        .find(|e| e.pseudonym_key == "peerA")
        .expect("peerA present");
    assert_eq!(peer_a.display_name.as_deref(), Some("Peer A"));
    assert!(entries.iter().any(|e| e.pseudonym_key == "me"));
}

#[test]
fn roster_does_not_duplicate_self_already_in_transport() {
    let peers = vec![("me".to_string(), vec![7], None)];
    let entries = build_roster_entries("me", Some(vec![1]), None, peers);
    assert_eq!(entries.len(), 1);
    // transport route is authoritative — self is not re-appended
    assert_eq!(entries[0].route_blob, vec![7]);
}

#[test]
fn roster_leaves_self_out_without_a_media_route() {
    let peers = vec![("peerA".to_string(), vec![9], None)];
    let entries = build_roster_entries("me", None, None, peers);
    assert_eq!(entries.len(), 1);
    assert!(entries.iter().all(|e| e.pseudonym_key != "me"));
}
