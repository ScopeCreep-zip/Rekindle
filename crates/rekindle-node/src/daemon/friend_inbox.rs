//! Friend inbox scanning — shared between tier 1 (watch), tier 2 (DM ack),
//! and tier 3 (poll) triggers.
//!
//! Uses `inspect()` to identify populated subkeys in one network call,
//! then reads only those subkeys. This avoids the 32 × 10s sequential
//! read penalty for empty subkeys that made scanning take 5+ minutes.

use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use tracing::{debug, info, warn, error};

use rekindle_transport::broadcast::node::TransportNode;
use rekindle_transport::payload::dht_types::{
    FriendRequestEntry, FriendRequestStatus, FRIEND_INBOX_SUBKEY_COUNT,
};
use rekindle_transport::session::{PendingFriendRequest, Session};

/// Scan the friend inbox DHT record for new pending requests.
///
/// 1. `inspect()` the record — one network call returns sequence numbers for all 32 subkeys
/// 2. Filter to subkeys with `seq.is_some()` (have been written)
/// 3. `get()` only populated subkeys — typically 1-4 network calls instead of 32
/// 4. Parse, filter Pending, skip already-known, persist new ones
///
/// Called from:
/// - `handler.rs::on_value_change` (tier 1: DHT watch fires)
/// - `handler.rs::on_dm` (tier 2: FriendRequestAck received)
/// - `node_daemon.rs` consumer task (tier 3: poll discovers change)
pub async fn scan_friend_inbox(
    session: &RwLock<Option<Session>>,
    transport: &RwLock<Option<Arc<TransportNode>>>,
    session_path: &std::path::Path,
    inbox_key: &str,
) {
    let start = Instant::now();
    info!(inbox_key = &inbox_key[..20.min(inbox_key.len())], "friend inbox scan: starting");

    let Some(transport_node) = transport.read().clone() else {
        error!("friend inbox scan: transport not available — cannot scan");
        return;
    };

    // Ensure record is open (idempotent for already-open records)
    if let Err(e) = rekindle_transport::broadcast::dht_writes::open_readonly(
        &transport_node, inbox_key,
    ).await {
        warn!(error = %e, "friend inbox scan: open_readonly failed");
    }

    // Step 1: Inspect — one network call to get all subkey sequence numbers.
    let all_subkeys: Vec<u32> = (0..FRIEND_INBOX_SUBKEY_COUNT).collect();
    let inspect_start = Instant::now();
    let report = match rekindle_transport::broadcast::dht_writes::inspect(
        &transport_node, inbox_key, Some(&all_subkeys),
    ).await {
        Ok(r) => {
            debug!(elapsed_ms = inspect_start.elapsed().as_millis(), "friend inbox scan: inspect complete");
            r
        }
        Err(e) => {
            warn!(
                error = %e,
                elapsed_ms = inspect_start.elapsed().as_millis(),
                "friend inbox scan: inspect failed, falling back to full scan (SLOW)"
            );
            scan_subkeys_direct(&transport_node, session, session_path, inbox_key, &all_subkeys, start).await;
            return;
        }
    };

    // Step 2: Filter to populated subkeys only
    let populated: Vec<u32> = report.subkeys().iter()
        .zip(report.local_seqs().iter())
        .filter(|(_, seq)| seq.is_some())
        .map(|(subkey, _)| subkey)
        .collect();

    if populated.is_empty() {
        info!(
            elapsed_ms = start.elapsed().as_millis(),
            "friend inbox scan: complete — no populated subkeys (inbox empty)"
        );
        return;
    }

    info!(
        populated = populated.len(),
        total = FRIEND_INBOX_SUBKEY_COUNT,
        skipped = FRIEND_INBOX_SUBKEY_COUNT as usize - populated.len(),
        "friend inbox scan: inspect found {} populated subkeys, skipping {} empty",
        populated.len(), FRIEND_INBOX_SUBKEY_COUNT as usize - populated.len()
    );

    // Step 3: Read only populated subkeys
    scan_subkeys_direct(&transport_node, session, session_path, inbox_key, &populated, start).await;
}

/// Read specific subkeys, parse friend requests, persist new ones.
async fn scan_subkeys_direct(
    transport_node: &TransportNode,
    session: &RwLock<Option<Session>>,
    session_path: &std::path::Path,
    inbox_key: &str,
    subkeys: &[u32],
    scan_start: Instant,
) {
    let mut found_new = 0u32;
    let mut found_known = 0u32;
    let mut found_non_pending = 0u32;
    let mut read_errors = 0u32;
    let mut parse_errors = 0u32;

    for &subkey in subkeys {
        let data = match rekindle_transport::broadcast::dht_writes::get(
            transport_node, inbox_key, subkey, true,
        ).await {
            Ok(Some(d)) if !d.is_empty() && d != b"[]" => d,
            Ok(_) => continue,
            Err(e) => {
                debug!(subkey, error = %e, "friend inbox scan: get failed for subkey");
                read_errors += 1;
                continue;
            }
        };

        // Parse as array first, fall back to single entry for backward compatibility
        let entries: Vec<FriendRequestEntry> = match serde_json::from_slice::<Vec<FriendRequestEntry>>(&data) {
            Ok(arr) => arr,
            Err(_) => match serde_json::from_slice::<FriendRequestEntry>(&data) {
                Ok(single) => vec![single],
                Err(e) => {
                    warn!(subkey, error = %e, bytes = data.len(), "friend inbox scan: parse failed");
                    parse_errors += 1;
                    continue;
                }
            },
        };

        for entry in entries {

        if !matches!(entry.status, FriendRequestStatus::Pending) {
            debug!(subkey, from = %entry.display_name, status = ?entry.status, "friend inbox scan: skipping non-pending");
            found_non_pending += 1;
            continue;
        }

        let already_known = {
            let guard = session.read();
            guard.as_ref().is_some_and(|s| {
                s.pending_friend_requests.iter().any(|r| r.public_key == entry.sender_public_key)
            })
        };
        if already_known {
            debug!(from = %entry.display_name, "friend inbox scan: already known, skipping");
            found_known += 1;
            continue;
        }

        let pending = PendingFriendRequest {
            public_key: entry.sender_public_key.clone(),
            display_name: entry.display_name.clone(),
            message: entry.message.clone(),
            profile_dht_key: entry.profile_dht_key.clone(),
            route_blob: Vec::new(),
            mailbox_dht_key: entry.mailbox_dht_key.clone(),
            prekey_bundle: entry.prekey_bundle.clone(),
            invite_id: None,
            received_at: entry.sent_at,
        };
        {
            let mut guard = session.write();
            if let Some(ref mut s) = *guard {
                s.add_pending_friend_request(pending);
            }
        }
        found_new += 1;
        info!(
            from = %entry.display_name,
            sender_key = &entry.sender_public_key[..16.min(entry.sender_public_key.len())],
            subkey,
            "friend inbox scan: NEW request discovered"
        );

        } // end for entry in entries
    }

    if found_new > 0 {
        let guard = session.read();
        if let Some(ref s) = *guard {
            if let Err(e) = s.save(session_path) {
                error!(error = %e, "friend inbox scan: session save FAILED after discovering requests");
            }
        }
    }

    info!(
        elapsed_ms = scan_start.elapsed().as_millis(),
        subkeys_read = subkeys.len(),
        new_requests = found_new,
        already_known = found_known,
        non_pending = found_non_pending,
        read_errors,
        parse_errors,
        "friend inbox scan: complete"
    );
}
