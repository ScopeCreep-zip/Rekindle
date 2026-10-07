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
fn silent_rostered_peer_is_reannounced_to() {
    // alice is in our roster and her row says she's in our channel, but
    // she is sending us no media (not media-live) — our route likely
    // never reached her. Re-announce.
    let roster = vec![("alice".to_string(), 100u64)];
    let out = compute_route_reannounce(
        "ch1",
        "me",
        &roster,
        &[row("alice", Some("ch1"), true)],
        &no_media(),
    );
    assert_eq!(out, vec!["alice".to_string()]);
}

#[test]
fn media_live_rostered_peer_is_not_reannounced_to() {
    // bob streams to us → he already holds our route → no retry.
    let roster = vec![("bob".to_string(), 100u64)];
    let out = compute_route_reannounce(
        "ch1",
        "me",
        &roster,
        &[row("bob", Some("ch1"), true)],
        &media(&["bob"]),
    );
    assert!(out.is_empty());
}

#[test]
fn not_yet_rostered_peer_is_an_introduction_not_a_reannounce() {
    // carol is not in the roster → that's an introduction, never a re-announce
    // (the two sets are disjoint).
    let out = compute_route_reannounce(
        "ch1",
        "me",
        &[],
        &[row("carol", Some("ch1"), true)],
        &no_media(),
    );
    assert!(out.is_empty());
}

#[test]
fn reannounce_skips_other_channels_and_self() {
    let roster = vec![("dave".to_string(), 100u64), ("me".to_string(), 100u64)];
    let out = compute_route_reannounce(
        "ch1",
        "me",
        &roster,
        &[
            row("dave", Some("ch2"), true), // different channel
            row("me", Some("ch1"), true),   // ourselves
        ],
        &no_media(),
    );
    assert!(out.is_empty());
}

#[test]
fn fresh_claim_for_our_channel_is_introduced() {
    let plan = compute_roster_reconcile(
        "ch1",
        "me",
        &[],
        &[row("alice", Some("ch1"), true)],
        &no_media(),
    );
    assert_eq!(plan.introduce, vec!["alice".to_string()]);
    assert!(plan.remove.is_empty());
}

#[test]
fn self_other_channel_and_stale_rows_are_not_introduced() {
    let plan = compute_roster_reconcile(
        "ch1",
        "me",
        &[],
        &[
            row("me", Some("ch1"), true),     // self
            row("bob", Some("ch2"), true),    // different channel
            row("carol", Some("ch1"), false), // stale heartbeat
            row("erin", None, true),          // not in any channel
        ],
        &no_media(),
    );
    assert!(plan.introduce.is_empty());
    assert!(plan.remove.is_empty());
}

#[test]
fn already_rostered_member_is_not_reintroduced() {
    let roster = vec![("alice".to_string(), 100u64)];
    let plan = compute_roster_reconcile(
        "ch1",
        "me",
        &roster,
        &[row("alice", Some("ch1"), true)],
        &no_media(),
    );
    assert!(plan.introduce.is_empty());
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
fn media_live_stale_row_is_introduced() {
    // The live-call bug, scenario (a): alice's audio is flowing to
    // us but her row is heartbeat-stale, so she never appeared in
    // the roster. Media liveness relaxes the freshness gate: our
    // re-sent join makes her ack with her route.
    let plan = compute_roster_reconcile(
        "ch1",
        "me",
        &[],
        &[row("alice", Some("ch1"), false)],
        &media(&["alice"]),
    );
    assert_eq!(plan.introduce, vec!["alice".to_string()]);
    assert!(plan.remove.is_empty());
}

#[test]
fn non_media_live_stale_row_behaves_as_before() {
    // No regression: media liveness for bob changes nothing about
    // carol, whose stale row is still not introduced (no roster entry)
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
    assert!(plan.introduce.is_empty()); // dave's stale row: no introduction
    assert_eq!(plan.remove, vec!["carol".to_string()]);
}
