//! Gossip payload dispatch — per-payload-type handlers extracted from
//! the monolithic `handle_gossip` method.
//!
//! The verification skeleton (envelope deserialize, signature verify,
//! inner payload deserialize) stays in `CommunityService::handle_gossip`.
//! This module handles everything after verification:
//! - Route blob extraction from gossip payloads
//! - TTL decrement + re-broadcast (gossip forwarding)
//! - ChannelMessage interception + MEK decrypt + vault store
//! - Control payload side-effects (MEK transfer, lockdown, etc.)
//! - Conversion to SubscriptionEvent

use parking_lot::RwLock;
use rekindle_storage::VaultStore;
use rekindle_types::gossip_payload::{GossipPayload, ControlPayload, SignedGossipEnvelope};
use rekindle_types::session_types::SessionMeta;
use rekindle_types::subscription_events::SubscriptionEvent;
use tracing::{debug, info, trace, warn};

use crate::crypto::mek::MekCache;
use crate::io::PlatformIO;

/// Dispatch a verified gossip payload to the appropriate handler.
///
/// Called by `CommunityService::handle_gossip` after signature verification.
/// All arguments are borrowed from CommunityService fields.
pub(crate) async fn dispatch_verified_gossip(
    io: &PlatformIO,
    vault: &VaultStore,
    mek_cache: &MekCache,
    session_meta: &RwLock<SessionMeta>,
    envelope: &SignedGossipEnvelope,
    payload: GossipPayload,
) -> Option<SubscriptionEvent> {
    let community = &envelope.community_id;
    let sender = &envelope.sender_pseudonym;

    let my_pseudonym = {
        let meta = session_meta.read();
        meta.communities.get(community)
            .map(|m| m.pseudonym_key.clone())
            .unwrap_or_default()
    };

    // ── Route blob extraction ──────────────────────────────────────
    extract_routes(io, envelope, &payload, &my_pseudonym);

    // ── Gossip forwarding (TTL decrement) ──────────────────────────
    if envelope.ttl > 1 {
        forward_with_ttl(io, envelope).await;
    }

    // ── ChannelMessage: MEK-decrypt before event emission ──────────
    if let GossipPayload::ChannelMessage {
        ref channel_id, ref message_id, ref ciphertext, mek_generation,
        sequence, timestamp, ref reply_to, ref thread_id,
    } = payload {
        info!(
            community = &community[..12.min(community.len())],
            sender = &sender[..16.min(sender.len())],
            channel = &channel_id[..12.min(channel_id.len())],
            message_id = &message_id[..12.min(message_id.len())],
            mek_generation,
            sequence,
            ciphertext_len = ciphertext.len(),
            "gossip_dispatch: ChannelMessage received — attempting MEK decrypt"
        );
        return handle_channel_message(
            vault, mek_cache, envelope,
            channel_id, message_id, ciphertext, mek_generation,
            sequence, timestamp, reply_to, thread_id,
        );
    }

    // ── Control payload side-effects ───────────────────────────────
    if let GossipPayload::Control(ref ctrl) = payload {
        handle_control_side_effects(session_meta, envelope, ctrl);
    }

    // ── Convert to SubscriptionEvent ───────────────────────────────
    let event = crate::events::conversions::gossip_to_event(payload, community, sender);

    debug!(
        community = &community[..12.min(community.len())],
        sender = &sender[..12.min(sender.len())],
        "gossip: verified and dispatched"
    );

    Some(event)
}

/// Extract route blobs from gossip payloads and cache them.
///
/// Route blobs carried in gossip are the sender's current private route.
/// Caching them keeps the peer registry and gossip mesh alive between
/// the 90s background populate cycles.
fn extract_routes(
    io: &PlatformIO,
    envelope: &SignedGossipEnvelope,
    payload: &GossipPayload,
    my_pseudonym: &str,
) {
    let community = &envelope.community_id;

    match payload {
        GossipPayload::PresenceUpdate { route_blob: Some(blob), pseudonym_key, status, .. } if !blob.is_empty() => {
            io.cache_peer_route(pseudonym_key, blob.clone());
            io.upsert_mesh_peer(community, pseudonym_key, blob.clone(), status, my_pseudonym);
            io.transport().register_peer_profile(pseudonym_key, community);
            trace!(
                pseudonym = &pseudonym_key[..16.min(pseudonym_key.len())],
                blob_len = blob.len(),
                "gossip: presence route cached + mesh upserted"
            );
        }
        GossipPayload::Control(ControlPayload::MemberJoined { pseudonym_key, route_blob: Some(blob), status, .. }) if !blob.is_empty() => {
            io.cache_peer_route(pseudonym_key, blob.clone());
            io.upsert_mesh_peer(community, pseudonym_key, blob.clone(), status, my_pseudonym);
            trace!(
                pseudonym = &pseudonym_key[..16.min(pseudonym_key.len())],
                "gossip: joined member route cached"
            );
        }
        GossipPayload::Control(ControlPayload::MemberJoinRequest { route_blob: Some(blob), pseudonym_key, .. }) if !blob.is_empty() => {
            io.cache_peer_route(pseudonym_key, blob.clone());
            trace!(
                pseudonym = &pseudonym_key[..16.min(pseudonym_key.len())],
                "gossip: join requester route cached"
            );
        }
        GossipPayload::Control(ControlPayload::MemberLeave { pseudonym_key }) => {
            io.remove_mesh_peer(community, pseudonym_key);
            io.invalidate_peer_route(pseudonym_key);
            debug!(
                pseudonym = &pseudonym_key[..16.min(pseudonym_key.len())],
                "gossip: member left — route invalidated"
            );
        }
        GossipPayload::Control(ControlPayload::MemberRemoved { pseudonym_key }) => {
            io.remove_mesh_peer(community, pseudonym_key);
            io.invalidate_peer_route(pseudonym_key);
            debug!(
                pseudonym = &pseudonym_key[..16.min(pseudonym_key.len())],
                "gossip: member removed — route invalidated"
            );
        }
        _ => {}
    }
}

/// Forward a gossip message with decremented TTL.
async fn forward_with_ttl(io: &PlatformIO, envelope: &SignedGossipEnvelope) {
    let mut fwd = envelope.clone();
    fwd.ttl -= 1;
    match postcard::to_stdvec(&fwd) {
        Ok(fwd_bytes) => {
            let mut framed = Vec::with_capacity(1 + fwd_bytes.len());
            framed.push(0x0A);
            framed.extend_from_slice(&fwd_bytes);
            let _ = io.broadcast_gossip_unmanaged(&envelope.community_id, &framed).await;
            trace!(
                community = &envelope.community_id[..12.min(envelope.community_id.len())],
                new_ttl = fwd.ttl,
                "gossip: forwarded with decremented TTL"
            );
        }
        Err(e) => {
            warn!(error = %e, "gossip forward serialization failed");
        }
    }
}

/// Handle a ChannelMessage gossip payload — MEK decrypt + vault store.
///
/// Returns the SubscriptionEvent directly (early return from dispatch).
fn handle_channel_message(
    vault: &VaultStore,
    mek_cache: &MekCache,
    envelope: &SignedGossipEnvelope,
    channel_id: &str,
    message_id: &str,
    ciphertext: &[u8],
    mek_generation: u64,
    sequence: u64,
    timestamp: u64,
    reply_to: &Option<u64>,
    thread_id: &Option<String>,
) -> Option<SubscriptionEvent> {
    let community = &envelope.community_id;

    let body = match mek_cache.get_generation(community, channel_id, mek_generation) {
        Some(mek_key) => {
            match crate::crypto::mek::mek_decrypt(&mek_key, ciphertext, &[]) {
                Ok(plaintext) => {
                    trace!(
                        channel = &channel_id[..12.min(channel_id.len())],
                        message_id = &message_id[..12.min(message_id.len())],
                        mek_generation,
                        plaintext_len = plaintext.len(),
                        "gossip: ChannelMessage decrypted"
                    );
                    Some(String::from_utf8(plaintext).unwrap_or_else(|_| "[binary]".into()))
                }
                Err(e) => {
                    warn!(
                        channel = &channel_id[..12.min(channel_id.len())],
                        mek_generation, error = %e,
                        "gossip: ChannelMessage MEK decrypt failed"
                    );
                    None
                }
            }
        }
        None => {
            debug!(
                channel = &channel_id[..12.min(channel_id.len())],
                mek_generation,
                "gossip: ChannelMessage MEK not cached — slow-path catch-up will deliver"
            );
            None
        }
    };

    // Persist to vault if decrypted
    if let Some(ref body_text) = body {
        let _ = vault.store_channel_message(
            community, channel_id, &envelope.sender_pseudonym, "",
            body_text, timestamp, sequence, message_id, mek_generation,
            *reply_to, thread_id.as_deref(),
        );
    }

    Some(SubscriptionEvent::ChannelMessage(
        rekindle_types::subscription_events::ChannelMessageEvent::New {
            community: community.to_string(),
            channel: channel_id.to_string(),
            message_id: message_id.to_string(),
            sender_pseudonym: envelope.sender_pseudonym.clone(),
            sequence,
            timestamp,
            body,
            reply_to_sequence: *reply_to,
            is_self: false,
            client_msg_id: None,
        },
    ))
}

/// Handle control payload side-effects that don't need CommunityService.
///
/// RequestMek and MekTransfer are handled by the caller (handle_gossip)
/// because they need `&CommunityService`. This function handles the
/// remaining side-effects that only need session_meta.
fn handle_control_side_effects(
    session_meta: &RwLock<SessionMeta>,
    envelope: &SignedGossipEnvelope,
    ctrl: &ControlPayload,
) {
    let community = &envelope.community_id;

    match ctrl {
        ControlPayload::ChannelLockdown { locked } => {
            let mut meta = session_meta.write();
            if let Some(membership) = meta.communities.get_mut(community) {
                membership.locked_down = *locked;
                info!(
                    community = &community[..12.min(community.len())],
                    locked,
                    "lockdown state updated from gossip"
                );
            }
        }
        _ => {}
    }
}
