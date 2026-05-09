//! Social dispatch handlers: Friend*, Dm*.

use std::sync::Arc;

use crate::daemon::DaemonState;
use crate::ipc::protocol::IpcResponse;
use crate::validation;

use super::{DaemonContext, state_error};

pub(crate) async fn handle_friend_add(
    ctx: &Arc<DaemonContext>, state: DaemonState, target: &str, message: &str,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    if let Err(e) = validation::validate_key(target, "target profile key") { return e; }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let signing_key = match ctx.require_signing_key() { Ok(k) => k, Err(e) => return e };
    let session = match ctx.require_session(Clone::clone) { Ok(s) => s, Err(e) => return e };

    match rekindle_transport::operations::friend::send_friend_request(
        &transport, &session, target, message, &signing_key,
    ).await {
        Ok(sent) => {
            // Persist prekey private material for X3DH completion across restarts.
            // Key label uses the TARGET's profile DHT key prefix. The acceptance
            // discovery path in friend_inbox.rs loads using entry.profile_dht_key
            // which is the same key (the acceptor's profile key = our original target).
            let target_short = if target.len() > 12 { &target[..12] } else { target };
            if !sent.signed_prekey_private.is_empty() {
                let _ = crate::state::keystore::store_keypair_bytes(
                    &format!("friend-spk-{target_short}"), &sent.signed_prekey_private,
                ).await;
            }
            if let Some(ref otpk) = sent.one_time_prekey_private {
                if !otpk.is_empty() {
                    let _ = crate::state::keystore::store_keypair_bytes(
                        &format!("friend-otpk-{target_short}"), otpk,
                    ).await;
                }
            }
            // Persist DM log keypair to keyring. The outbound log key is stored
            // in pending_outbound_logs keyed by the target's profile DHT key.
            // When the acceptance is discovered (friend_inbox.rs), we learn the
            // peer's Ed25519 key and migrate this to dm_peers under the correct key.
            let log_short = if sent.dm_log_key.len() > 12 { &sent.dm_log_key[..12] } else { &sent.dm_log_key };
            let _ = crate::state::keystore::store_keypair_bytes(
                &format!("dm-log-{log_short}"), &sent.dm_log_keypair_bytes,
            ).await;
            {
                let mut guard = ctx.session.write();
                if let Some(ref mut s) = *guard {
                    s.pending_outbound_logs.insert(target.to_string(), sent.dm_log_key.clone());
                }
            }
            if let Err(e) = ctx.save_session() { return e; }

            IpcResponse::ok(&serde_json::json!({ "status": "sent", "target": target, "dm_log_key": sent.dm_log_key }))
        }
        Err(e) => IpcResponse::error(500, format!("friend request failed: {e}")),
    }
}

pub(crate) async fn handle_friend_accept(
    ctx: &Arc<DaemonContext>, state: DaemonState, public_key: &str,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };

    // Already friends? Both sent requests and one was already accepted.
    {
        let guard = ctx.session.read();
        if let Some(ref s) = *guard {
            if let Some(peer_log) = s.dm_peers.get(public_key) {
                if !peer_log.outbound_log_key.is_empty() && !peer_log.inbound_log_key.is_empty() {
                    return IpcResponse::ok(&serde_json::json!({
                        "accepted": public_key,
                        "outbound_log": peer_log.outbound_log_key,
                        "inbound_log": peer_log.inbound_log_key,
                        "already_friends": true,
                    }));
                }
            }
        }
    }

    let (session, request) = {
        // Check session for existing pending request first (no async needed)
        let found = {
            let guard = ctx.session.read();
            let Some(session) = guard.as_ref() else {
                return IpcResponse::error(404, "no identity loaded");
            };
            session.pending_request_by_key(public_key)
                .map(|req| (session.clone(), req.clone()))
        };
        if let Some(result) = found {
            result
        } else {
            // Not in session yet — targeted subkey read with retry.
            //
            // Instead of scanning all 32 inbox subkeys via inspect() (10s+ network
            // round-trip), compute the exact subkey where the requester wrote their
            // entry and read it directly with force_refresh=true. This is a single
            // network call per attempt instead of 33+.
            let (inbox_key, our_profile_key) = {
                let guard = ctx.session.read();
                let Some(s) = guard.as_ref() else {
                    return IpcResponse::error(404, "no identity loaded");
                };
                (s.identity.friend_inbox_key.clone(), s.identity.profile_dht_key.clone())
            };

            // The requester wrote to our inbox at this subkey:
            // blake3_hash_mod(requester_pubkey, our_profile_key, 32)
            let target_subkey = rekindle_transport::operations::friend::blake3_hash_mod(
                public_key, &our_profile_key, 32,
            );

            tracing::info!(
                public_key = &public_key[..16.min(public_key.len())],
                target_subkey,
                "friend accept: request not in session, reading inbox subkey directly"
            );

            let scan_start = std::time::Instant::now();
            let scan_deadline = std::time::Duration::from_secs(30);
            let mut attempt = 0u32;

            // Ensure inbox record is open for reading
            if let Ok(t) = ctx.require_transport() {
                let _ = rekindle_transport::broadcast::dht_writes::open_readonly(&t, &inbox_key).await;
            }

            while scan_start.elapsed() < scan_deadline {
                // Direct get with force_refresh — single network call
                if let Ok(t) = ctx.require_transport() {
                    if let Ok(Some(data)) = rekindle_transport::broadcast::dht_writes::get(
                        &t, &inbox_key, target_subkey, true,
                    ).await {
                        if !data.is_empty() && data != b"[]" {
                            // Parse entries directly — check for Accepted (mutual) or Pending
                            use rekindle_transport::payload::dht_types::{FriendRequestEntry, FriendRequestStatus};
                            let entries = FriendRequestEntry::parse_inbox_data(&data).unwrap_or_default();

                            // Check for mutual acceptance: they already accepted OUR request.
                            // With sender-creates protocol, the dm_log_key in their Accepted
                            // entry is the one WE created when we sent our request — we already
                            // have it in our session. Just confirm the friendship.
                            if let Some(accepted_entry) = entries.iter().find(|e| {
                                e.sender_public_key == public_key
                                    && matches!(e.status, FriendRequestStatus::Accepted { .. })
                            }) {
                                if let FriendRequestStatus::Accepted {
                                    ref responder_outbound_log_key, ..
                                } = accepted_entry.status {
                                    tracing::info!(
                                        public_key = &public_key[..16.min(public_key.len())],
                                        elapsed_secs = scan_start.elapsed().as_secs(),
                                        "friend accept: mutual — they already accepted our request"
                                    );

                                    // The responder's outbound log = our inbound.
                                    // Our outbound was stored in pending_outbound_logs at send time.
                                    // Migrate to dm_peers under the Ed25519 key.
                                    {
                                        let mut guard = ctx.session.write();
                                        if let Some(ref mut s) = *guard {
                                            let display_name = s.pending_request_by_key(public_key)
                                                .map(|r| r.display_name.clone())
                                                .unwrap_or_default();
                                            s.remove_pending_friend_request(public_key);
                                            if !display_name.is_empty() {
                                                s.friend_display_names.insert(public_key.to_string(), display_name);
                                            }
                                            let our_outbound = s.pending_outbound_logs
                                                .remove(&accepted_entry.profile_dht_key)
                                                .unwrap_or_default();
                                            let peer_log = s.dm_peers.entry(public_key.to_string()).or_insert_with(|| {
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
                                    if let Err(e) = ctx.save_session() { return e; }

                                    // Watch the inbound log for DM receipt
                                    {
                                        let watch_deps = {
                                            let guard = ctx.subscriptions.read();
                                            guard.as_ref().map(|mgr| (
                                                std::sync::Arc::clone(mgr.node()),
                                                std::sync::Arc::clone(mgr.watches()),
                                            ))
                                        };
                                        let peer = public_key.to_string();
                                        let inbound_log = responder_outbound_log_key.clone();
                                        tokio::spawn(async move {
                                            if let Some((node, watches)) = watch_deps {
                                                rekindle_transport::subscriptions::watches::setup_dm_watch(
                                                    &node, &watches, &peer, &inbound_log,
                                                ).await;
                                                tracing::info!(peer = &peer[..16.min(peer.len())], "DM inbound watch established (mutual accept)");
                                            }
                                        });
                                    }

                                    return IpcResponse::ok(&serde_json::json!({
                                        "accepted": public_key,
                                        "inbound_log": responder_outbound_log_key,
                                        "mutual": true,
                                    }));
                                }
                            }

                            // Normal path: scan for Pending entries and persist to session
                            let transport_node = match ctx.require_transport() {
                                Ok(t) => t,
                                Err(e) => return e,
                            };
                            crate::daemon::friend_inbox::scan_friend_inbox(
                                &ctx.session, &transport_node, &ctx.session_path, &inbox_key, &ctx.signal,
                            ).await;

                            let found = ctx.session.read().as_ref()
                                .and_then(|s| s.pending_request_by_key(public_key).cloned())
                                .is_some();
                            if found {
                                tracing::info!(
                                    public_key = &public_key[..16.min(public_key.len())],
                                    elapsed_secs = scan_start.elapsed().as_secs(),
                                    attempt,
                                    "friend accept: request discovered via targeted subkey read"
                                );
                                break;
                            }
                        }
                    }
                }

                attempt += 1;
                // Backoff: 2s, 4s, 8s
                let wait = std::time::Duration::from_secs(2u64.saturating_pow(attempt).min(8));
                tokio::time::sleep(wait).await;
            }

            // Re-read session after targeted scan
            let guard = ctx.session.read();
            let Some(session) = guard.as_ref() else {
                return IpcResponse::error(404, "no identity loaded");
            };
            let Some(req) = session.pending_request_by_key(public_key) else {
                return IpcResponse::error(404, format!(
                    "no pending request from {} — they may not have sent one, or DHT propagation is still in progress",
                    &public_key[..16.min(public_key.len())],
                ));
            };
            (session.clone(), req.clone())
        }
    };

    let signing_key = match ctx.require_signing_key() { Ok(k) => k, Err(e) => return e };

    // Establish Signal session on the daemon's persistent manager BEFORE
    // calling accept_friend_request. The session init info is included in
    // the Accepted entry so the sender can complete their side.
    //
    // MUTUAL CASE: If a Signal session already exists for this peer (because
    // the peer already accepted OUR request and we ran respond_to_session),
    // do NOT overwrite it with a new establish_session. The existing session
    // is from a completed X3DH handshake — replacing it would create an
    // incompatible session where encrypt/decrypt fails on both sides.
    let (signal_init, identity_public_key) = {
        let bundle = match rekindle_transport::crypto::prekeys::PreKeyBundle::from_bytes(&request.prekey_bundle) {
            Ok(b) => b,
            Err(e) => return IpcResponse::error(500, format!("invalid prekey bundle: {e}")),
        };
        let guard = ctx.signal.read();
        let Some(ref signal_mgr) = *guard else {
            return IpcResponse::error(500, "Signal session manager not initialized");
        };
        let already_has = signal_mgr.has_session(public_key).unwrap_or(false);
        let init = if already_has {
            tracing::info!(
                peer = &public_key[..16.min(public_key.len())],
                "Signal session already exists (mutual accept) — keeping existing session"
            );
            // Return empty init info — the peer already has their session
            // established via their own establish_session + our respond_to_session.
            // Empty ephemeral_public_key causes the peer's inbox scan to skip
            // respond_to_session entirely (guard: !ephemeral_public_key.is_empty()).
            // This is fail-closed: we never write fake handshake data that could
            // be mistaken for a valid X3DH exchange.
            rekindle_transport::crypto::signal_session::SessionInitInfo {
                ephemeral_public_key: Vec::new(),
                signed_prekey_id: 0,
                one_time_prekey_id: None,
            }
        } else {
            match signal_mgr.establish_session(public_key, &bundle) {
                Ok(info) => {
                    tracing::info!(
                        peer = &public_key[..16.min(public_key.len())],
                        "Signal session established (acceptor side, persistent manager)"
                    );
                    info
                }
                Err(e) => return IpcResponse::error(500, format!("Signal session establishment failed: {e}")),
            }
        };
        // Derive our X25519 identity public key for the Accepted entry
        let id_pub = rekindle_transport::crypto::signal_session::identity_public_key_bytes(&signing_key);
        (init, id_pub)
    };

    match rekindle_transport::operations::friend::accept_friend_request(
        &transport, &session, &request.public_key,
        &request.route_blob, &request.profile_dht_key, &request.display_name,
        &request.dm_log_key, &request.dm_log_keypair_hex,
        &signing_key, &signal_init, &identity_public_key,
    ).await {
        Ok(accepted) => {
            // Store both DM log keypairs in OS keyring
            let out_short = if accepted.outbound_log_key.len() > 12 { &accepted.outbound_log_key[..12] } else { &accepted.outbound_log_key };
            let _ = crate::state::keystore::store_keypair_bytes(
                &format!("dm-log-{out_short}"), &accepted.outbound_log_keypair_bytes,
            ).await;
            let in_short = if accepted.inbound_log_key.len() > 12 { &accepted.inbound_log_key[..12] } else { &accepted.inbound_log_key };
            let _ = crate::state::keystore::store_keypair_bytes(
                &format!("dm-log-{in_short}"), &accepted.inbound_log_keypair_bytes,
            ).await;

            // Update session: set up both outbound + inbound logs.
            // Also clean up any pending_outbound_logs entry if we had sent a
            // request to this peer too (mutual case — our outbound from that
            // request is orphaned since we're using the one from accept_friend_request).
            {
                let mut guard = ctx.session.write();
                if let Some(ref mut s) = *guard {
                    s.friend_display_names.insert(public_key.to_string(), request.display_name.clone());
                    s.remove_pending_friend_request(public_key);
                    s.pending_outbound_logs.remove(&request.profile_dht_key);
                    let peer_log = s.dm_peers.entry(public_key.to_string()).or_insert_with(|| {
                        rekindle_transport::session::DmPeerLog {
                            outbound_log_key: String::new(),
                            inbound_log_key: String::new(),
                        }
                    });
                    peer_log.outbound_log_key.clone_from(&accepted.outbound_log_key);
                    peer_log.inbound_log_key.clone_from(&accepted.inbound_log_key);
                }
            }
            if let Err(e) = ctx.save_session() { return e; }

            // Watch the inbound log (peer's outbound → our inbound) for real-time DM receipt
            {
                let watch_deps = {
                    let guard = ctx.subscriptions.read();
                    guard.as_ref().map(|mgr| (
                        std::sync::Arc::clone(mgr.node()),
                        std::sync::Arc::clone(mgr.watches()),
                    ))
                };
                let peer = public_key.to_string();
                let inbound_log = accepted.inbound_log_key.clone();
                tokio::spawn(async move {
                    if let Some((node, watches)) = watch_deps {
                        rekindle_transport::subscriptions::watches::setup_dm_watch(
                            &node, &watches, &peer, &inbound_log,
                        ).await;
                        tracing::info!(peer = &peer[..16.min(peer.len())], "DM inbound watch established post-accept");
                    }
                });
            }

            IpcResponse::ok(&serde_json::json!({
                "accepted": public_key,
                "outbound_log": accepted.outbound_log_key,
                "inbound_log": accepted.inbound_log_key,
            }))
        }
        Err(e) => IpcResponse::error(500, format!("friend accept failed: {e}")),
    }
}

pub(crate) async fn handle_friend_reject(
    ctx: &Arc<DaemonContext>, state: DaemonState, public_key: &str,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let signing_key = match ctx.require_signing_key() { Ok(k) => k, Err(e) => return e };

    let session = match ctx.require_session(Clone::clone) { Ok(s) => s, Err(e) => return e };

    match rekindle_transport::operations::friend::reject_friend_request(
        &transport, &session, public_key, &signing_key,
    ).await {
        Ok(()) => {
            {
                let mut guard = ctx.session.write();
                if let Some(ref mut s) = *guard {
                    s.remove_pending_friend_request(public_key);
                }
            }
            if let Err(e) = ctx.save_session() { return e; }
            IpcResponse::ok(&serde_json::json!({ "rejected": public_key }))
        }
        Err(e) => IpcResponse::error(500, format!("friend reject failed: {e}")),
    }
}

pub(crate) async fn handle_friend_remove(
    ctx: &Arc<DaemonContext>, state: DaemonState, public_key: &str,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let session = match ctx.require_session(Clone::clone) { Ok(s) => s, Err(e) => return e };

    match rekindle_transport::operations::friend::remove_friend(
        &transport, &session, public_key,
    ).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "removed": public_key })),
        Err(e) => IpcResponse::error(500, format!("friend remove failed: {e}")),
    }
}

pub(crate) async fn handle_friend_list(
    ctx: &Arc<DaemonContext>, state: DaemonState,
) -> IpcResponse {
    if !state.can_query() { return state_error(state, "query"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let (friend_list_key, our_key) = match ctx.require_session(|s| {
        (s.identity.friend_list_dht_key.clone(), s.identity.public_key_hex.clone())
    }) { Ok(t) => t, Err(e) => return e };
    let query = match transport.query(Arc::clone(&ctx.mek_cache), Arc::clone(&ctx.signal)) {
        Ok(q) => q, Err(e) => return IpcResponse::error(500, format!("query engine: {e}")),
    };
    match query.resolved_friends(&friend_list_key).await {
        Ok(friends) => {
            // Filter out the local user's own key (defensive — shouldn't be in the list
            // but can appear due to bidirectional friend request flows)
            let filtered: Vec<_> = friends.into_iter().filter(|f| f.public_key != our_key).collect();
            IpcResponse::ok(&filtered)
        }
        Err(e) => IpcResponse::error(500, format!("friend list: {e}")),
    }
}

pub(crate) fn handle_friend_requests(ctx: &Arc<DaemonContext>, state: DaemonState) -> IpcResponse {
    if !state.can_query() { return state_error(state, "query"); }
    ctx.require_session(|session| {
        IpcResponse::ok(&session.pending_friend_requests)
    }).unwrap_or_else(|e| e)
}

pub(crate) async fn handle_dm_send(
    ctx: &Arc<DaemonContext>, state: DaemonState, peer_key: &str, body: &str,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    if let Err(e) = validation::validate_message_body(body) { return e; }
    if let Err(e) = validation::validate_key(peer_key, "peer key") { return e; }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let signing_key = match ctx.require_signing_key() { Ok(k) => k, Err(e) => return e };
    let session = match ctx.require_session(Clone::clone) { Ok(s) => s, Err(e) => return e };

    // DMs require an established friendship with an outbound DhtLog.
    if !session.dm_peers.contains_key(peer_key) {
        return IpcResponse::error(404, format!(
            "cannot DM '{}' — not a friend. Accept their request or add them first: rekindle friend add",
            &peer_key[..16.min(peer_key.len())],
        ));
    }

    // Load outbound DM log keypair from keyring (we write to our outbound log)
    let dm_log_keypair_bytes = if let Some(peer_log) = session.dm_peers.get(peer_key) {
        if peer_log.outbound_log_key.is_empty() {
            None
        } else {
            let short = if peer_log.outbound_log_key.len() > 12 { &peer_log.outbound_log_key[..12] } else { &peer_log.outbound_log_key };
            let label = format!("dm-log-{short}");
            match crate::state::keystore::load_keypair_bytes(&label).await {
                Ok(Some(bytes)) => Some(bytes),
                _ => None,
            }
        }
    } else {
        None
    };

    // Encrypt body with Signal Protocol before writing to DhtLog.
    // The Signal session encrypts with the sending chain key and performs
    // a DH ratchet step, providing forward secrecy per message.
    let encrypted_body = {
        let guard = ctx.signal.read();
        if let Some(ref signal_mgr) = *guard {
            match signal_mgr.encrypt(peer_key, body.as_bytes()) {
                Ok(ciphertext) => ciphertext,
                Err(e) => {
                    return IpcResponse::error(500, format!(
                        "Signal encrypt failed for {}: {e} — re-establish friendship to reset session",
                        &peer_key[..16.min(peer_key.len())],
                    ));
                }
            }
        } else {
            return IpcResponse::error(500,
                "Signal session manager not initialized — unlock the daemon before sending DMs",
            );
        }
    };

    match rekindle_transport::operations::dm::send_dm(
        &transport, &session, peer_key, &encrypted_body, &signing_key, dm_log_keypair_bytes.as_deref(),
    ).await {
        Ok(()) => {
            let dm_msg_id = format!("dm-{}", uuid::Uuid::now_v7());
            let timestamp = rekindle_transport::timestamp_ms();

            // Persist sent plaintext to local encrypted store.
            // Forward secrecy means we cannot re-decrypt our own outbound
            // ciphertext from the DhtLog — this is the only copy of the plaintext.
            {
                let mut store = ctx.local_messages.lock();
                if let Some(ref mut s) = *store {
                    s.store_sent(peer_key, body, timestamp, &dm_msg_id);
                }
            }

            // Emit local subscription event so the sender's own TUI sees
            // the DM via the same pipeline as the recipient. Only fires
            // after the DHT write succeeded.
            {
                let subs = ctx.subscriptions.read();
                if let Some(ref mgr) = *subs {
                    let sender_name = session.identity.display_name.clone();
                    mgr.emit_local(rekindle_types::subscription_events::SubscriptionEvent::ChannelMessage(
                        rekindle_types::subscription_events::ChannelMessageEvent::DirectMessageReceived {
                            peer_key: peer_key.to_string(),
                            timestamp: rekindle_transport::timestamp_ms(),
                            sender_name: Some(sender_name),
                            body: Some(body.to_string()),
                            is_self: true,
                        },
                    ));
                }
            }
            IpcResponse::ok(&serde_json::json!({ "status": "sent", "peer_key": peer_key, "message_id": dm_msg_id }))
        }
        Err(e) => {
            tracing::error!(peer = peer_key, error = %e, "dm send failed");
            IpcResponse::error(500, format!("dm send failed: {e}"))
        }
    }
}

pub(crate) async fn handle_dm_typing(
    ctx: &Arc<DaemonContext>, state: DaemonState, peer_key: &str, typing: bool,
) -> IpcResponse {
    if !state.can_write() { return state_error(state, "write"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let signing_key = match ctx.require_signing_key() { Ok(k) => k, Err(e) => return e };
    let session = match ctx.require_session(Clone::clone) { Ok(s) => s, Err(e) => return e };

    match rekindle_transport::operations::dm::send_typing(
        &transport, &session, peer_key, typing, &signing_key,
    ).await {
        Ok(()) => IpcResponse::ok(&serde_json::json!({ "typing": typing })),
        Err(e) => IpcResponse::error(500, format!("typing indicator failed: {e}")),
    }
}

pub(crate) async fn handle_dm_inbox(
    ctx: &Arc<DaemonContext>, state: DaemonState, limit: u32,
) -> IpcResponse {
    if !state.can_query() { return state_error(state, "query"); }
    let transport = match ctx.require_transport() { Ok(t) => t, Err(e) => return e };
    let (dm_peers, friend_list_key, our_key, session_display_names) = match ctx.require_session(|s| {
        (
            s.dm_peers.clone(),
            s.identity.friend_list_dht_key.clone(),
            s.identity.public_key_hex.clone(),
            s.friend_display_names.clone(),
        )
    }) { Ok(t) => t, Err(e) => return e };

    if dm_peers.is_empty() {
        return IpcResponse::ok(&serde_json::json!([]));
    }

    // Read both outbound + inbound DhtLogs per peer and aggregate into threads
    let mut all_threads = Vec::new();
    let query = match transport.query(Arc::clone(&ctx.mek_cache), Arc::clone(&ctx.signal)) {
        Ok(q) => q, Err(e) => return IpcResponse::error(500, format!("query engine: {e}")),
    };

    for (peer_key, peer_log) in &dm_peers {
        // Read sent messages from local encrypted store (NOT from outbound DhtLog —
        // forward secrecy means we cannot decrypt our own outbound ciphertext).
        {
            let mut store = ctx.local_messages.lock();
            if let Some(ref mut s) = *store {
                let mut sent = s.query_sent(peer_key, limit as usize);
                // Fill in our public key for each sent message
                for msg in &mut sent {
                    msg.sender_key.clone_from(&our_key);
                }
                if !sent.is_empty() {
                    let last_at = sent.last().map_or(0, |m| m.timestamp);
                    let peer_name = session_display_names.get(peer_key)
                        .cloned()
                        .unwrap_or_else(|| if peer_key.len() > 12 {
                            format!("{}…{}", &peer_key[..8], &peer_key[peer_key.len()-4..])
                        } else {
                            peer_key.clone()
                        });
                    all_threads.push(rekindle_transport::query::DmThreadDisplay {
                        peer_key: peer_key.clone(),
                        peer_name,
                        last_message_at: last_at,
                        unread_count: 0,
                        messages: sent,
                    });
                }
            }
        }

        // Read received messages from inbound DhtLog via Signal decrypt.
        if !peer_log.inbound_log_key.is_empty() {
            match query.dm_inbox(&peer_log.inbound_log_key, &friend_list_key, limit as usize, &our_key, &session_display_names).await {
                Ok(mut threads) => all_threads.append(&mut threads),
                Err(e) => {
                    tracing::debug!(peer = %&peer_key[..12.min(peer_key.len())], error = %e, "inbound DM log read failed");
                }
            }
        }
    }

    // Sort by most recent message first
    all_threads.sort_by(|a: &rekindle_transport::query::DmThreadDisplay, b| {
        b.last_message_at.cmp(&a.last_message_at)
    });

    IpcResponse::ok(&all_threads)
}
