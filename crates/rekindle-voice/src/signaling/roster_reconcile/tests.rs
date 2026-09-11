//! Unit tests for the presence-derived voice roster reconcile.
//!
//! Split out to keep `mod.rs` under the file-size ceiling; this is
//! the `#[cfg(test)] mod tests;` sibling of `roster_reconcile/mod.rs`.

use std::collections::HashSet;

use super::*;

fn row(pseudonym: &str, channel: Option<&str>, fresh: bool) -> PresencePeerView {
    PresencePeerView {
        pseudonym_hex: pseudonym.to_string(),
        display_name: Some(format!("{pseudonym}-name")),
        route_blob: vec![1, 2, 3],
        voice_channel_id: channel.map(str::to_string),
        fresh,
    }
}

/// No media evidence for anyone — the pre-media-liveness behavior.
fn no_media() -> HashSet<String> {
    HashSet::new()
}

fn media(peers: &[&str]) -> HashSet<String> {
    peers.iter().map(|p| (*p).to_string()).collect()
}

#[test]
fn fresh_claim_for_our_channel_is_added() {
    let plan = compute_roster_reconcile(
        "ch1",
        "me",
        &[],
        &[row("alice", Some("ch1"), true)],
        &no_media(),
    );
    assert_eq!(plan.add.len(), 1);
    assert_eq!(plan.add[0].pseudonym_hex, "alice");
    assert_eq!(plan.add[0].display_name.as_deref(), Some("alice-name"));
    assert!(plan.remove.is_empty());
}

#[test]
fn self_other_channel_stale_and_routeless_rows_are_not_added() {
    let mut routeless = row("dave", Some("ch1"), true);
    routeless.route_blob.clear();
    let plan = compute_roster_reconcile(
        "ch1",
        "me",
        &[],
        &[
            row("me", Some("ch1"), true),     // self
            row("bob", Some("ch2"), true),    // different channel
            row("carol", Some("ch1"), false), // stale heartbeat
            row("erin", None, true),          // not in any channel
            routeless,                        // no route yet
        ],
        &no_media(),
    );
    assert!(plan.add.is_empty());
    assert!(plan.remove.is_empty());
}

#[test]
fn already_rostered_member_is_not_readded() {
    let roster = vec![("alice".to_string(), 100u64)];
    let plan = compute_roster_reconcile(
        "ch1",
        "me",
        &roster,
        &[row("alice", Some("ch1"), true)],
        &no_media(),
    );
    assert!(plan.add.is_empty());
    assert!(plan.remove.is_empty());
}

#[test]
fn join_grace_protects_fresh_roster_entries() {
    // alice's row freshly says she left — but she was added 10s
    // ago via gossip; her presence row may simply predate her join.
    let roster = vec![("alice".to_string(), 10u64)];
    let plan = compute_roster_reconcile(
        "ch1",
        "me",
        &roster,
        &[row("alice", None, true)],
        &no_media(),
    );
    assert!(plan.remove.is_empty());
}

#[test]
fn fresh_row_claiming_elsewhere_expires_aged_entry() {
    let roster = vec![("alice".to_string(), JOIN_GRACE_SECS + 1)];
    let plan = compute_roster_reconcile(
        "ch1",
        "me",
        &roster,
        &[row("alice", Some("ch2"), true)],
        &no_media(),
    );
    assert_eq!(plan.remove, vec!["alice".to_string()]);
}

#[test]
fn stale_heartbeat_expires_aged_entry() {
    let roster = vec![("alice".to_string(), JOIN_GRACE_SECS + 1)];
    let plan = compute_roster_reconcile(
        "ch1",
        "me",
        &roster,
        &[row("alice", Some("ch1"), false)],
        &no_media(),
    );
    assert_eq!(plan.remove, vec!["alice".to_string()]);
}

#[test]
fn missing_presence_row_never_expires_a_peer() {
    let roster = vec![("alice".to_string(), 10_000u64)];
    let plan = compute_roster_reconcile("ch1", "me", &roster, &[], &no_media());
    assert!(plan.remove.is_empty());
}

#[test]
fn media_live_peer_with_stale_row_is_not_removed() {
    // The live-call bug, scenario (b): alice's audio is flowing but
    // her DHT presence writes are failing, so her row is stale.
    // Media outranks the directory — she stays.
    let roster = vec![("alice".to_string(), JOIN_GRACE_SECS + 1)];
    let plan = compute_roster_reconcile(
        "ch1",
        "me",
        &roster,
        &[row("alice", Some("ch1"), false)],
        &media(&["alice"]),
    );
    assert!(plan.remove.is_empty());
}

#[test]
fn media_live_peer_freshly_claiming_elsewhere_is_not_removed() {
    // Even a fresh row claiming another channel loses to packets
    // arriving NOW — the media veto covers the `left` branch too.
    let roster = vec![("alice".to_string(), JOIN_GRACE_SECS + 1)];
    let plan = compute_roster_reconcile(
        "ch1",
        "me",
        &roster,
        &[row("alice", Some("ch2"), true)],
        &media(&["alice"]),
    );
    assert!(plan.remove.is_empty());
}

#[test]
fn media_live_stale_row_is_added() {
    // The live-call bug, scenario (a): alice's audio is flowing to
    // us but her row is heartbeat-stale, so she never appeared in
    // the roster. Media liveness relaxes the freshness gate — the
    // stale row still carries the route blob we need.
    let plan = compute_roster_reconcile(
        "ch1",
        "me",
        &[],
        &[row("alice", Some("ch1"), false)],
        &media(&["alice"]),
    );
    assert_eq!(plan.add.len(), 1);
    assert_eq!(plan.add[0].pseudonym_hex, "alice");
    assert!(plan.remove.is_empty());
}

#[test]
fn non_media_live_stale_row_behaves_as_before() {
    // No regression: media liveness for bob changes nothing about
    // carol, whose stale row is still not added (no roster entry)
    // and still expires her aged roster entry.
    let roster = vec![("carol".to_string(), JOIN_GRACE_SECS + 1)];
    let plan = compute_roster_reconcile(
        "ch1",
        "me",
        &roster,
        &[
            row("carol", Some("ch1"), false),
            row("dave", Some("ch1"), false),
        ],
        &media(&["bob"]),
    );
    assert!(plan.add.is_empty()); // dave's stale row: still no add
    assert_eq!(plan.remove, vec!["carol".to_string()]);
}

/// A presence row with a caller-chosen route blob (the `row` helper
/// hardcodes `[1,2,3]`; supersession tests need to vary it).
fn row_blob(pseudonym: &str, channel: Option<&str>, fresh: bool, blob: &[u8]) -> PresencePeerView {
    PresencePeerView {
        route_blob: blob.to_vec(),
        ..row(pseudonym, channel, fresh)
    }
}

#[test]
fn changed_blob_for_rostered_peer_is_refreshed() {
    // The restart heal: alice is rostered with an old blob; her
    // presence row now carries a new one. Supersede it.
    let current = vec![("alice".to_string(), vec![1u8, 1, 1])];
    let refreshes = compute_blob_refreshes(
        "ch1",
        "me",
        &current,
        &[row_blob("alice", Some("ch1"), true, &[2, 2, 2])],
        &no_media(),
    );
    assert_eq!(refreshes.len(), 1);
    assert_eq!(refreshes[0].pseudonym_hex, "alice");
    assert_eq!(refreshes[0].route_blob, vec![2, 2, 2]);
}

#[test]
fn unchanged_blob_is_not_refreshed() {
    // Steady state: the row's blob matches what we already send to.
    let current = vec![("alice".to_string(), vec![9u8, 9, 9])];
    let refreshes = compute_blob_refreshes(
        "ch1",
        "me",
        &current,
        &[row_blob("alice", Some("ch1"), true, &[9, 9, 9])],
        &no_media(),
    );
    assert!(refreshes.is_empty());
}

#[test]
fn media_live_stale_row_still_supersedes_blob() {
    // A peer whose DHT writes are failing (stale row) but whose audio
    // is flowing can still re-announce a new route — the supersede
    // must fire on the media-live path, not only the fresh path.
    let current = vec![("alice".to_string(), vec![1u8])];
    let refreshes = compute_blob_refreshes(
        "ch1",
        "me",
        &current,
        &[row_blob("alice", Some("ch1"), false, &[2])],
        &media(&["alice"]),
    );
    assert_eq!(refreshes.len(), 1);
}

#[test]
fn non_rostered_or_self_or_wrong_channel_is_not_refreshed() {
    // Not held yet → that's an ADD, not a refresh. Self and
    // other-channel rows never refresh. Routeless never refreshes.
    let current = vec![("alice".to_string(), vec![1u8])];
    let refreshes = compute_blob_refreshes(
        "ch1",
        "me",
        &current,
        &[
            row_blob("bob", Some("ch1"), true, &[2]), // not in current_blobs
            row_blob("me", Some("ch1"), true, &[2]),  // self
            row_blob("alice", Some("ch2"), true, &[2]), // different channel
            row_blob("alice", Some("ch1"), true, &[]), // routeless
        ],
        &no_media(),
    );
    assert!(refreshes.is_empty());
}
