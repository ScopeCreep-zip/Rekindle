//! Friend inbox scanning — shared between tier 1 (watch), tier 2 (DM ack),
//! and tier 3 (poll) triggers.
//!
//! Uses `inspect()` to identify populated subkeys in one network call,
//! then reads only those subkeys. This avoids the 32 × 10s sequential
//! read penalty for empty subkeys that made scanning take 5+ minutes.
//!
//! Scanning is coordinated by `InboxScanCoordinator` which coalesces
//! redundant triggers and enforces a cooldown between scans. Callers
//! use `coordinator.trigger()` (non-blocking channel send) instead of
//! calling `scan_friend_inbox()` directly.

use std::sync::Arc;
use std::time::Instant;

use parking_lot::RwLock;
use tracing::{debug, info, warn, error};

use rekindle_transport::broadcast::node::TransportNode;
use rekindle_transport::payload::dht_types::{
    FriendRequestEntry, FriendRequestStatus, FRIEND_INBOX_SUBKEY_COUNT,
};
use rekindle_transport::session::{PendingFriendRequest, Session};

/// Minimum interval between consecutive inbox scans.
const SCAN_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);

/// Coordinates friend inbox scans from multiple trigger sources.
///
/// Multiple triggers (tier 1 watch, tier 2 DM ack, tier 3 poll) may fire
/// within seconds of each other for the same inbox change. The coordinator
/// coalesces these into a single scan with a 30-second cooldown.
///
/// `trigger()` is non-blocking (`try_send`) — callers never wait for a scan.
pub struct InboxScanCoordinator {
    trigger_tx: tokio::sync::mpsc::Sender<String>,
}

impl InboxScanCoordinator {
    /// Create a new coordinator that runs scans on a dedicated background task.
    ///
    /// Takes a snapshot of the current transport node. The transport Arc is
    /// cloned once — the coordinator holds it for the lifetime of the task.
    ///
    /// `subscriptions` is used to call `setup_dm_peer` when acceptances are
    /// discovered — establishing the DHT watch so DMs are received in real-time.
    pub fn spawn(
        session: Arc<RwLock<Option<Session>>>,
        transport: Arc<TransportNode>,
        session_path: std::path::PathBuf,
        subscriptions: Arc<RwLock<Option<rekindle_transport::SubscriptionManager>>>,
        signal: Arc<parking_lot::RwLock<Option<rekindle_transport::crypto::signal_session::SignalSessionManager>>>,
    ) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(4);

        tokio::spawn(async move {
            let mut last_scan = Instant::now().checked_sub(SCAN_COOLDOWN).unwrap_or_else(Instant::now);

            while let Some(inbox_key) = rx.recv().await {
                // Coalesce: skip if scanned recently
                if last_scan.elapsed() < SCAN_COOLDOWN {
                    debug!(
                        cooldown_remaining_secs = (SCAN_COOLDOWN - last_scan.elapsed()).as_secs(),
                        "inbox scan coalesced — scanned recently"
                    );
                    // Drain any queued duplicates
                    while rx.try_recv().is_ok() {}
                    continue;
                }

                let new_acceptances = scan_friend_inbox(&session, &transport, &session_path, &inbox_key, &signal).await;
                last_scan = Instant::now();

                // Set up DM watches for newly discovered acceptances.
                // Extract the node + watches Arcs from SubscriptionManager, drop
                // the parking_lot guard, THEN await the DHT operations. This avoids
                // holding a parking_lot lock across an await point (which would block
                // the OS thread — see DaemonContext lock strategy in dispatch/mod.rs).
                for acceptance in &new_acceptances {
                    let watch_deps = {
                        let guard = subscriptions.read();
                        guard.as_ref().map(|mgr| (
                            Arc::clone(mgr.node()),
                            Arc::clone(mgr.watches()),
                        ))
                    };
                    // Guard dropped — safe to await
                    if let Some((node, watches)) = watch_deps {
                        rekindle_transport::subscriptions::watches::setup_dm_watch(
                            &node, &watches, &acceptance.peer_key, &acceptance.dm_log_key,
                        ).await;
                        info!(
                            peer = &acceptance.peer_key[..16.min(acceptance.peer_key.len())],
                            "DM watch established (acceptance discovered via inbox scan)"
                        );
                    }
                }

                // Drain any triggers that arrived during the scan
                while rx.try_recv().is_ok() {}
            }
        });

        Self { trigger_tx: tx }
    }

    /// Request an inbox scan. Non-blocking — returns immediately.
    ///
    /// If the channel is full (4 pending triggers), the trigger is dropped.
    /// This is safe because the coordinator drains duplicates after each scan.
    pub fn trigger(&self, inbox_key: &str) {
        let _ = self.trigger_tx.try_send(inbox_key.to_string());
    }
}

/// Scan the friend inbox DHT record for new pending requests and acceptances.
///
/// 1. `inspect()` the record — one network call returns sequence numbers for all 32 subkeys
/// 2. Filter to subkeys with `seq.is_some()` (have been written)
/// 3. `get()` only populated subkeys — typically 1-4 network calls instead of 32
/// 4. Parse entries: Pending → persist as new request, Accepted → store DM log key
///
/// Returns newly discovered acceptances so callers can set up DM watches.
///
/// Called from:
/// - `handler.rs::on_value_change` (tier 1: DHT watch fires)
/// - `handler.rs::on_dm` (tier 2: FriendRequestAck received)
/// - `node_daemon.rs` consumer task (tier 3: poll discovers change)
pub async fn scan_friend_inbox(
    session: &RwLock<Option<Session>>,
    transport_node: &TransportNode,
    session_path: &std::path::Path,
    inbox_key: &str,
    signal: &parking_lot::RwLock<Option<rekindle_transport::crypto::signal_session::SignalSessionManager>>,
) -> Vec<DiscoveredAcceptance> {
    let start = Instant::now();
    info!(inbox_key = &inbox_key[..20.min(inbox_key.len())], "friend inbox scan: starting");

    // Ensure record is open (idempotent for already-open records)
    if let Err(e) = rekindle_transport::broadcast::dht_writes::open_readonly(
        transport_node, inbox_key,
    ).await {
        warn!(error = %e, "friend inbox scan: open_readonly failed");
    }

    // Step 1: Inspect — one network call to get all subkey sequence numbers.
    let all_subkeys: Vec<u32> = (0..FRIEND_INBOX_SUBKEY_COUNT).collect();
    let inspect_start = Instant::now();
    let report = match rekindle_transport::broadcast::dht_writes::inspect(
        transport_node, inbox_key, Some(&all_subkeys),
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
            return scan_subkeys_direct(transport_node, session, session_path, inbox_key, &all_subkeys, start, signal).await;
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
        return Vec::new();
    }

    info!(
        populated = populated.len(),
        total = FRIEND_INBOX_SUBKEY_COUNT,
        skipped = FRIEND_INBOX_SUBKEY_COUNT as usize - populated.len(),
        "friend inbox scan: inspect found {} populated subkeys, skipping {} empty",
        populated.len(), FRIEND_INBOX_SUBKEY_COUNT as usize - populated.len()
    );

    // Step 3: Read only populated subkeys
    scan_subkeys_direct(transport_node, session, session_path, inbox_key, &populated, start, signal).await
}

/// A newly discovered acceptance from the friend inbox scan.
/// The caller uses this to set up the DM watch via SubscriptionManager.
pub struct DiscoveredAcceptance {
    pub peer_key: String,
    pub dm_log_key: String,
}

/// Read specific subkeys, parse friend requests and acceptances, persist new ones.
///
/// Returns a list of newly discovered acceptances so callers can set up DM watches.
async fn scan_subkeys_direct(
    transport_node: &TransportNode,
    session: &RwLock<Option<Session>>,
    session_path: &std::path::Path,
    inbox_key: &str,
    subkeys: &[u32],
    scan_start: Instant,
    signal: &parking_lot::RwLock<Option<rekindle_transport::crypto::signal_session::SignalSessionManager>>,
) -> Vec<DiscoveredAcceptance> {
    let mut found_new = 0u32;
    let mut found_known = 0u32;
    let mut found_non_pending = 0u32;
    let mut found_acceptances = 0u32;
    let mut read_errors = 0u32;
    let mut parse_errors = 0u32;
    let mut new_acceptances = Vec::new();

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

        let entries: Vec<FriendRequestEntry> = match FriendRequestEntry::parse_inbox_data(&data) {
            Ok(e) => e,
            Err(e) => {
                warn!(subkey, error = %e, bytes = data.len(), "friend inbox scan: parse failed");
                parse_errors += 1;
                continue;
            }
        };

        for entry in entries {

        // ── Signature verification ──────────────────────────────────
        // Every entry must be signed by the sender's Ed25519 key.
        // Reject unsigned or invalid entries to prevent impersonation
        // on the world-writable friend inbox record.
        let sig_valid = if entry.signature_hex.is_empty() {
            false
        } else {
            (|| -> Option<bool> {
                let sig_bytes = hex::decode(&entry.signature_hex).ok()?;
                let pub_bytes = hex::decode(&entry.sender_public_key).ok()?;
                let sig_arr: [u8; 64] = sig_bytes.try_into().ok()?;
                let pub_arr: [u8; 32] = pub_bytes.try_into().ok()?;
                let vk = ed25519_dalek::VerifyingKey::from_bytes(&pub_arr).ok()?;
                let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);
                use ed25519_dalek::Verifier;
                vk.verify(&entry.signature_content(), &sig).ok()?;
                Some(true)
            })().unwrap_or(false)
        };
        if !sig_valid {
            warn!(
                from = %entry.display_name, subkey,
                sender = &entry.sender_public_key[..16.min(entry.sender_public_key.len())],
                "friend inbox scan: rejecting entry — signature invalid or missing"
            );
            parse_errors += 1;
            continue;
        }

        // ── Process Accepted responses ──────────────────────────────
        // The sender already has the dm_log_key (stored at send time).
        // The Accepted entry confirms the friendship. Clean up pending
        // state and set up the display name + DM watch.
        if let FriendRequestStatus::Accepted {
            ref responder_outbound_log_key,
            ref responder_identity_key,
            ref ephemeral_public_key,
            signed_prekey_id,
            one_time_prekey_id,
            ..
        } = entry.status {
            let already_processed = {
                let guard = session.read();
                guard.as_ref().is_some_and(|s| s.dm_peers.contains_key(&entry.sender_public_key))
            };
            // The responder's outbound log = our inbound (we read their messages).
            // Our outbound log was stored in pending_outbound_logs keyed by the
            // acceptor's profile DHT key. Migrate it to dm_peers under the
            // acceptor's Ed25519 key (the correct SSOT key for dm_peers).
            {
                let mut guard = session.write();
                if let Some(ref mut s) = *guard {
                    s.friend_display_names.insert(entry.sender_public_key.clone(), entry.display_name.clone());
                    s.remove_pending_friend_request(&entry.sender_public_key);
                    // Recover our outbound log key from the pending send
                    let our_outbound = s.pending_outbound_logs
                        .remove(&entry.profile_dht_key)
                        .unwrap_or_default();
                    let peer_log = s.dm_peers.entry(entry.sender_public_key.clone()).or_insert_with(|| {
                        rekindle_transport::session::DmPeerLog {
                            outbound_log_key: String::new(),
                            inbound_log_key: String::new(),
                        }
                    });
                    if !our_outbound.is_empty() {
                        peer_log.outbound_log_key = our_outbound;
                    }
                    peer_log.inbound_log_key.clone_from(responder_outbound_log_key);
                }
            }
            // Establish sender-side Signal session (respond_to_session).
            // The responder ran establish_session (initiator X3DH) during accept.
            // Now the sender responds using the handshake fields from the Accepted entry
            // + their prekey private material from the keyring.
            if !ephemeral_public_key.is_empty() && !responder_identity_key.is_empty() {
                // Check if a Signal session already exists for this peer.
                // In the bidirectional friend request case (both sent requests),
                // the acceptor side already called establish_session. Calling
                // respond_to_session here would overwrite it with a different
                // shared secret, breaking all subsequent encrypt/decrypt.
                let already_has_session = {
                    let guard = signal.read();
                    guard.as_ref().is_some_and(|mgr| mgr.has_session(&entry.sender_public_key).unwrap_or(false))
                };
                if already_has_session {
                    debug!(
                        peer = &entry.sender_public_key[..12.min(entry.sender_public_key.len())],
                        "Signal session already exists — skipping respond_to_session"
                    );
                } else {
                // Prekeys were stored under the TARGET's profile DHT key prefix
                // at send time (handle_friend_add). The acceptor's profile_dht_key
                // in the entry IS that target profile key.
                let target_short = &entry.profile_dht_key[..12.min(entry.profile_dht_key.len())];
                let spk_data = crate::state::keystore::load_keypair_bytes(
                    &format!("friend-spk-{target_short}")
                ).await.ok().flatten();

                if let Some(spk_bytes) = spk_data {
                    let otpk_data = if one_time_prekey_id.is_some() {
                        crate::state::keystore::load_keypair_bytes(
                            &format!("friend-otpk-{target_short}")
                        ).await.ok().flatten()
                    } else {
                        None
                    };

                    // Inject prekeys into the shared Signal session manager,
                    // then call respond_to_session. The session state lands in
                    // the KeyringSessionStore (persisted to OS keyring).
                    let sig_guard = signal.write();
                    if let Some(ref signal_mgr) = *sig_guard {
                        if let Err(e) = signal_mgr.inject_signed_prekey(signed_prekey_id, &spk_bytes) {
                            warn!(error = %e, "failed to inject signed prekey");
                        }
                        if let (Some(otpk_id), Some(ref otpk_bytes)) = (one_time_prekey_id, &otpk_data) {
                            if let Err(e) = signal_mgr.inject_prekey(otpk_id, otpk_bytes) {
                                warn!(error = %e, "failed to inject one-time prekey");
                            }
                        }
                        match signal_mgr.respond_to_session(
                            &entry.sender_public_key,
                            responder_identity_key,
                            ephemeral_public_key,
                            signed_prekey_id,
                            one_time_prekey_id,
                        ) {
                            Ok(()) => info!(
                                peer = target_short,
                                "Signal session established (sender side, on acceptance discovery)"
                            ),
                            Err(e) => warn!(
                                error = %e, peer = target_short,
                                "Signal respond_to_session failed — DMs will fail to decrypt"
                            ),
                        }
                    }
                } else {
                    warn!(
                        peer = &entry.sender_public_key[..12.min(entry.sender_public_key.len())],
                        "prekey material not found in keyring — cannot establish Signal session"
                    );
                }
                } // end else (no existing session)
            }

            if !already_processed {
                found_new += 1;
            }
            found_acceptances += 1;
            new_acceptances.push(DiscoveredAcceptance {
                peer_key: entry.sender_public_key.clone(),
                dm_log_key: responder_outbound_log_key.clone(),
            });
            info!(
                from = %entry.display_name,
                sender_key = &entry.sender_public_key[..16.min(entry.sender_public_key.len())],
                inbound_log = %&responder_outbound_log_key[..16.min(responder_outbound_log_key.len())],
                subkey,
                "friend inbox scan: ACCEPTANCE confirmed — inbound log stored"
            );
            continue;
        }

        // ── Skip non-pending entries ────────────────────────────────
        if !matches!(entry.status, FriendRequestStatus::Pending) {
            debug!(subkey, from = %entry.display_name, status = ?entry.status, "friend inbox scan: skipping non-pending");
            found_non_pending += 1;
            continue;
        }

        // ── Check if already friends ────────────────────────────────
        // If we already have a dm_log_key for this peer, the friendship
        // is resolved (we accepted their request, or they accepted ours).
        // This handles the "both sent requests" case — first accept wins.
        let already_friends = {
            let guard = session.read();
            guard.as_ref().is_some_and(|s| s.dm_peers.contains_key(&entry.sender_public_key))
        };
        if already_friends {
            debug!(from = %entry.display_name, "friend inbox scan: already friends, skipping pending request");
            found_non_pending += 1;
            continue;
        }

        // ── Check if already known pending ──────────────────────────
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

        // ── Persist new pending request ─────────────────────────────
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
            dm_log_key: entry.dm_log_key.clone(),
            dm_log_keypair_hex: entry.dm_log_keypair_hex.clone(),
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
        // Clone session and drop lock before filesystem I/O.
        // Session::save() does atomic_write (create + write + rename + fsync)
        // which must not block the parking_lot read guard.
        let session_snapshot = session.read().clone();
        if let Some(ref s) = session_snapshot {
            if let Err(e) = s.save(session_path) {
                error!(error = %e, "friend inbox scan: session save FAILED after discovering requests");
            }
        }
    }

    info!(
        elapsed_ms = scan_start.elapsed().as_millis(),
        subkeys_read = subkeys.len(),
        new_requests = found_new,
        new_acceptances = found_acceptances,
        already_known = found_known,
        non_pending = found_non_pending,
        read_errors,
        parse_errors,
        "friend inbox scan: complete"
    );

    new_acceptances
}
