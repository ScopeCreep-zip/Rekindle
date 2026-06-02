//! Community gossip broadcast — every GossipPayload and ControlPayload variant.
//!
//! Each public wrapper:
//! 1. Builds the typed `GossipPayload`
//! 2. Serializes via postcard
//! 3. Increments the community Lamport clock
//! 4. Signs with the community pseudonym Ed25519 key
//! 5. Fans out to mesh peers via `Sender::broadcast_gossip`
//!
//! Wrappers are grouped by domain into submodules and re-exported here, so the
//! public path `broadcast::gossip::<name>` is unchanged. Rate limits are enforced
//! for ephemeral signals (typing, presence); persistent signals are never limited.

use std::collections::HashMap;

use parking_lot::RwLock;
use tracing::{debug, trace, warn};

use super::node::TransportNode;
use super::send::BroadcastReport;
use crate::crypto::envelope;
use crate::gossip::GossipMesh;
use crate::payload::gossip::{ControlPayload, GossipPayload};

mod bootstrap_sync;
mod events;
mod keys;
mod membership;
mod messaging;
mod moderation;
mod signals;
mod voice;

pub use bootstrap_sync::{bootstrap_request, bootstrap_response, sync_request, sync_response};
pub use events::{
    event_created, event_deleted, event_reminder, event_rsvp_changed, event_updated,
    game_server_added, game_server_removed, thread_archived, thread_created, thread_message,
};
pub use keys::{
    admin_keypair_grant, governance_updated, mek_rotated, mek_transfer, request_mek,
    slot_keypair_grant,
};
pub use membership::{
    channel_overwrite_changed, join_accepted, join_rejected, member_join_request, member_joined,
    member_leave, member_removed, member_roles_changed, onboarding_complete,
    submit_onboarding_answers,
};
pub use messaging::{
    message_deleted, message_edited, message_pinned, message_unpinned, reaction_added,
    reaction_removed,
};
pub use moderation::{
    ban, channel_lockdown, kick, kicked_notification, member_timed_out, raid_alert, remove_timeout,
    system_message, timeout_member, unban,
};
pub use signals::{message_notification, presence_update, typing_indicator};
pub use voice::{
    voice_deafen, voice_join, voice_leave, voice_mode_switch, voice_mute, voice_roster,
};

/// Type alias for the community gossip mesh map to satisfy clippy::implicit_hasher.
pub type MeshMap = HashMap<String, GossipMesh>;

/// Default TTL for gossip broadcasts.
const DEFAULT_TTL: u8 = 3;

/// Shortcut for broadcasting a ControlPayload wrapped in GossipPayload::Control.
async fn control(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender: &str,
    signing_key: &[u8; 32],
    ctrl: ControlPayload,
) -> BroadcastReport {
    build_sign_send(
        node,
        meshes,
        community_id,
        sender,
        signing_key,
        GossipPayload::Control(ctrl),
    )
    .await
}

/// Build a signed gossip envelope and fan out to mesh peers.
async fn build_sign_send(
    node: &TransportNode,
    meshes: &RwLock<MeshMap>,
    community_id: &str,
    sender_pseudonym: &str,
    signing_key: &[u8; 32],
    payload: GossipPayload,
) -> BroadcastReport {
    // Serialize inner payload
    let payload_bytes = match postcard::to_stdvec(&payload) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "gossip broadcast: payload serialization failed");
            return BroadcastReport {
                delivered: 0,
                failures: vec![("*".into(), format!("serialize: {e}"))],
            };
        }
    };

    // Increment Lamport clock and collect peer route blobs
    let (lamport_ts, peer_targets) = {
        let mut guard = meshes.write();
        let Some(mesh) = guard.get_mut(community_id) else {
            debug!(community_id, "gossip broadcast: no mesh for community");
            return BroadcastReport::default();
        };
        let ts = mesh.clock.increment();
        let targets: Vec<(String, Vec<u8>)> = mesh
            .peers
            .iter()
            .map(|(k, m)| (k.clone(), m.route_blob.clone()))
            .collect();
        (ts, targets)
    };

    if peer_targets.is_empty() {
        trace!(community_id, "gossip broadcast: no peers in mesh");
        return BroadcastReport::default();
    }

    // Sign the envelope
    let signing = ed25519_dalek::SigningKey::from_bytes(signing_key);
    let envelope = envelope::sign_gossip_envelope(
        &signing,
        community_id,
        sender_pseudonym,
        &payload_bytes,
        DEFAULT_TTL,
        lamport_ts,
    );

    // Build framed targets and broadcast
    let sender = node.sender();
    let mut targets_with_routes = Vec::with_capacity(peer_targets.len());
    for (key, blob) in &peer_targets {
        match node.import_route(blob) {
            Ok(target) => targets_with_routes.push((key.clone(), target)),
            Err(e) => debug!(peer = %key, error = %e, "gossip broadcast: route import failed"),
        }
    }

    sender
        .broadcast_gossip(&targets_with_routes, &envelope)
        .await
}

/// Send a signed gossip envelope directly to a single target via their route.
///
/// Used for point-to-point notifications (e.g., JoinAccepted to a specific joiner)
/// where the target is NOT in the gossip mesh. Bypasses mesh lookup entirely.
///
/// This is the tier 2 (direct notification) primitive for authoritative state changes.
pub async fn send_direct(
    node: &TransportNode,
    community_id: &str,
    sender_pseudonym: &str,
    signing_key: &[u8; 32],
    payload: GossipPayload,
    target_key: &str,
    target_route_blob: &[u8],
) -> BroadcastReport {
    let payload_bytes = match postcard::to_stdvec(&payload) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "direct gossip: payload serialization failed");
            return BroadcastReport {
                delivered: 0,
                failures: vec![("*".into(), format!("serialize: {e}"))],
            };
        }
    };

    let signing = ed25519_dalek::SigningKey::from_bytes(signing_key);
    let envelope = envelope::sign_gossip_envelope(
        &signing,
        community_id,
        sender_pseudonym,
        &payload_bytes,
        0,
        0, // TTL=0 (no forwarding), lamport=0 (single-shot)
    );

    let target = match node.import_route(target_route_blob) {
        Ok(t) => t,
        Err(e) => {
            debug!(target = target_key, error = %e, "direct gossip: route import failed");
            return BroadcastReport {
                delivered: 0,
                failures: vec![(target_key.into(), format!("{e}"))],
            };
        }
    };

    node.sender()
        .broadcast_gossip(&[(target_key.to_string(), target)], &envelope)
        .await
}
