//! Phase 21 REDO — community-registry write orchestrators.
//!
//! Pre-port lived in
//! `src-tauri/services/community/presence/registry.rs`. Owns
//! `write_our_presence` (build + W11.2 encrypt + W26 sign + DHT
//! write) and `persist_discovered_registry_members` (diff + emit
//! MemberDiscovered + batch SQLite upsert + delete banned rows).

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

use rekindle_types::presence::MemberPresence;

use crate::community::time::now_secs;
use crate::deps::{CommunityPresenceDeps, DiscoveredMemberRow};

/// `(segment_index, local_subkey, presence)` tuple yielded by the
/// per-segment registry scan. Plate Gate (architecture §15) carries
/// segment context so downstream SQLite persistence preserves the
/// member's location.
pub type DiscoveredRow = (u32, u32, MemberPresence);

/// Inputs for [`write_our_presence`]. Groups the registry addressing +
/// slot credentials + history ranges so the entry point stays under the
/// argument-count budget while each caller-side lookup remains explicit
/// and auditable at the construction site.
pub struct PresenceWrite<'a> {
    pub community_id: &'a str,
    pub registry_key: &'a str,
    pub my_pseudonym_hex: &'a str,
    pub my_subkey_index: Option<u32>,
    pub slot_keypair_str: Option<&'a str>,
    pub has_slot_seed: bool,
    pub history_ranges: Vec<rekindle_types::presence::HistoryRange>,
}

/// Build a fully-signed presence row + write it to our subkey on
/// the registry record. Skips silently when credentials / writer
/// keypair / subkey index are missing — same semantics as the
/// pre-port `write_our_presence`.
pub async fn write_our_presence<D: CommunityPresenceDeps>(deps: &D, write: PresenceWrite<'_>) {
    let PresenceWrite {
        community_id,
        registry_key,
        my_pseudonym_hex,
        my_subkey_index,
        slot_keypair_str,
        has_slot_seed,
        history_ranges,
    } = write;
    let (Some(subkey_idx), Some(kp_str)) = (my_subkey_index, slot_keypair_str) else {
        tracing::warn!(
            community = %community_id,
            has_slot_keypair = slot_keypair_str.is_some(),
            has_subkey_index = my_subkey_index.is_some(),
            has_slot_seed,
            "cannot write presence — missing slot keypair or subkey index",
        );
        return;
    };

    let our_route_blob = deps.our_route_blob();
    if our_route_blob.is_none() {
        tracing::warn!(
            community = %community_id,
            "write_our_presence: our_route_blob is None — peers cannot reach us",
        );
    }

    let snapshot = deps.self_presence_snapshot(community_id);
    let history_ranges_encrypted = if history_ranges.is_empty() {
        None
    } else {
        deps.encrypt_history_ranges_with_current_mek(community_id, &history_ranges)
    };

    let pseudonym_bytes = hex::decode(my_pseudonym_hex)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
        .unwrap_or([0u8; 32]);

    // Typed session is the single source of truth: the loose `status`
    // wire string is derived from it so transitional readers and
    // session-aware readers can never disagree. Invisible folds to
    // "offline" in the wire string (peers must not see it).
    //
    // Privacy gate (default-deny): redact the session to exactly what our
    // sharing policy permits BEFORE signing, then move the surviving
    // identity-revealing signals (location/activity) out of the plaintext
    // into a MEK-encrypted blob so only current members can read them.
    let policy = deps.presence_policy(community_id);
    let mut session =
        crate::community::policy::apply_sharing_policy(deps.self_session(community_id), &policy);
    let extras = rekindle_types::presence::SessionExtras {
        location: session.location.take(),
        activity: session.activity.take(),
    };
    let session_extras_encrypted = if extras.location.is_some() || extras.activity.is_some() {
        deps.encrypt_session_extras_with_current_mek(community_id, &extras)
    } else {
        None
    };
    let status = session.status.as_wire_str().to_string();

    let mut presence = MemberPresence {
        pseudonym_key: rekindle_types::id::PseudonymKey(pseudonym_bytes),
        display_name: Some(deps.identity_display_name()),
        status,
        route_blob: our_route_blob.unwrap_or_default(),
        last_heartbeat: now_secs(),
        event_rsvps: snapshot.event_rsvps,
        history_ranges_encrypted,
        bio: snapshot.bio,
        pronouns: snapshot.pronouns,
        theme_color: snapshot.theme_color,
        badges: snapshot.badges,
        avatar_ref: snapshot.avatar_ref,
        banner_ref: snapshot.banner_ref,
        session,
        session_extras_encrypted,
        ..Default::default()
    };

    // Architecture §26 W26 — sign before publishing so receivers can
    // verify the presence row actually came from `pseudonym_key`.
    let Some(signature) = deps.sign_presence_row(community_id, &presence.signing_bytes()) else {
        tracing::warn!(
            community = %community_id,
            "skipping presence write: pseudonym credentials unavailable",
        );
        return;
    };
    presence.signature = signature;

    let presence_json = match serde_json::to_vec(&presence) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(
                community = %community_id,
                %error,
                "presence row serialisation failed",
            );
            return;
        }
    };

    if let Err(error) = deps
        .write_presence_to_registry_subkey(registry_key, subkey_idx, presence_json, kp_str)
        .await
    {
        tracing::debug!(
            community = %community_id,
            subkey = subkey_idx,
            %error,
            "failed to write presence to registry",
        );
    }
}

/// Diff against `known_members` to emit `MemberDiscovered` events
/// for newly-seen pseudonyms, then ask the host to upsert every row
/// (and delete banned-out members) in one batched SQLite transaction.
pub fn persist_discovered_registry_members<D, S1, S2>(
    deps: &D,
    community_id: &str,
    discovered_members: &[DiscoveredRow],
    member_roles: &HashMap<String, Vec<u32>, S1>,
    banned_members: &HashSet<String, S2>,
) where
    D: CommunityPresenceDeps,
    S1: BuildHasher,
    S2: BuildHasher,
{
    let rows: Vec<DiscoveredMemberRow> = discovered_members
        .iter()
        .map(|(segment_index, subkey, presence)| {
            build_member_row(presence, *segment_index, *subkey, member_roles)
        })
        .collect();

    // Detect newly-seen pseudonyms and fire one `MemberDiscovered`
    // event per row before the SQLite write so the UI can react
    // immediately. The host's `extend_known_members` returns just
    // the previously-unknown subset and atomically extends the
    // in-memory set.
    let candidates: Vec<String> = rows.iter().map(|r| r.pseudonym_key.clone()).collect();
    let newly_discovered: HashSet<String> = deps
        .extend_known_members(community_id, candidates)
        .into_iter()
        .collect();

    for row in &rows {
        if newly_discovered.contains(&row.pseudonym_key) {
            deps.emit_member_discovered(
                community_id,
                &row.pseudonym_key,
                row.display_name.as_deref().unwrap_or_default(),
                u32::try_from(row.subkey_index).unwrap_or(0),
            );
        }
    }

    let banned: Vec<String> = banned_members.iter().cloned().collect();
    let joined_at_secs = i64::try_from(now_secs()).unwrap_or(0);
    deps.persist_discovered_member_rows(community_id, rows, banned, joined_at_secs);
}

fn build_member_row<S: BuildHasher>(
    presence: &MemberPresence,
    segment_index: u32,
    subkey: u32,
    member_roles: &HashMap<String, Vec<u32>, S>,
) -> DiscoveredMemberRow {
    let pseudonym_hex = hex::encode(presence.pseudonym_key.0);
    let role_ids_json = serde_json::to_string(
        member_roles
            .get(&pseudonym_hex)
            .cloned()
            .unwrap_or_else(|| vec![0])
            .as_slice(),
    )
    .unwrap_or_else(|_| "[0]".to_string());
    let badges_json = serde_json::to_string(&presence.badges).unwrap_or_else(|_| "[]".to_string());
    DiscoveredMemberRow {
        pseudonym_key: pseudonym_hex,
        display_name: presence.display_name.clone(),
        role_ids_json,
        subkey_index: i64::from(subkey),
        segment_index: i64::from(segment_index),
        bio: presence.bio.clone(),
        pronouns: presence.pronouns.clone(),
        theme_color: presence.theme_color.map(i64::from),
        badges_json,
        avatar_ref: presence.avatar_ref.clone(),
        banner_ref: presence.banner_ref.clone(),
    }
}
