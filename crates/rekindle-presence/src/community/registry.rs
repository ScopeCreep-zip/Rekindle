//! Phase 21 REDO — community-registry write orchestrators.
//!
//! Pre-port lived in
//! `src-tauri/services/community/presence/registry.rs`. Owns
//! `write_our_presence` (build + W11.2 encrypt + W26 sign + DHT
//! write) and `persist_discovered_registry_members` (diff + emit
//! MemberDiscovered + batch SQLite upsert + delete banned rows).

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;

use rekindle_records::lease::{max_subkey_bytes, SchemaShape};
use rekindle_types::presence::{EncryptedHistoryRanges, HistoryRange, MemberPresence};

use crate::community::scan_row::SUBKEYS_PER_SEGMENT;
use crate::community::time::now_secs;
use crate::deps::{CommunityPresenceDeps, DiscoveredMemberRow, RowWrite};
use rekindle_codec::presence_row::{classify_superseding_row, SupersedingRow};

/// The most a presence row may encode to: one subkey of the SMPL(0,
/// 255×1) registry, 4112 bytes (plan V2, C7.15). `RecordPool::set`
/// refuses anything larger before Veilid sees it.
pub const PRESENCE_ROW_CAP: usize = max_subkey_bytes(SchemaShape {
    subkey_count: SUBKEYS_PER_SEGMENT,
});

/// An Ed25519 signature; its base64 form is always 88 characters, so a
/// row measured with a placeholder of this length measures as signed.
const SIGNATURE_LEN: usize = 64;

/// The encoded size `presence` will have once signed.
fn signed_len(presence: &MemberPresence) -> usize {
    let mut probe = presence.clone();
    probe.signature = vec![0; SIGNATURE_LEN];
    serde_json::to_vec(&probe).map_or(usize::MAX, |bytes| bytes.len())
}

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
    // Voice-roster membership claim (MatrixRTC `m.rtc.member` pattern) —
    // CLEARTEXT on the presence row, not in the MEK-encrypted extras.
    //
    // Which voice channel a member is in must be readable by every
    // member the moment they arrive, *before* the channel MEK has
    // converged. Behind the MEK it was not: a joining member with a
    // split-brained MEK (different bytes at the same generation, which
    // happens routinely on join) could not decrypt peers' extras, so
    // `voice_channel_id` read as None, the roster reconcile saw nobody
    // claiming the channel, and voice discovery stalled until the MEK
    // healed — minutes, or never. Discovery must not depend on media-key
    // convergence; MatrixRTC keeps call membership in cleartext room
    // state and E2EEs only the media, for exactly this reason. The row
    // is already members-only (W26-signed, in the SMPL registry) and
    // already carries `in_call`/`call_type` in cleartext, so the channel
    // id adds no meaningful exposure.
    let active_voice_channel = deps.active_voice_channel(community_id);
    let status = session.status.as_wire_str().to_string();

    let mut presence = MemberPresence {
        pseudonym_key: rekindle_types::id::PseudonymKey(pseudonym_bytes),
        display_name: Some(deps.identity_display_name()),
        status,
        route_blob: our_route_blob.unwrap_or_default(),
        last_heartbeat: now_secs(),
        bio: snapshot.bio,
        pronouns: snapshot.pronouns,
        theme_color: snapshot.theme_color,
        badges: snapshot.badges,
        avatar_ref: snapshot.avatar_ref,
        banner_ref: snapshot.banner_ref,
        session,
        session_extras_encrypted,
        voice_channel_id: active_voice_channel,
        ..Default::default()
    };

    // The fixed fields are bounded (`presence::limits`, proven by
    // `worst_case_row_fits_its_slot`); a row over the cap here means a
    // field escaped its bound, and the write would be refused anyway.
    let fixed_len = signed_len(&presence);
    if fixed_len > PRESENCE_ROW_CAP {
        tracing::warn!(
            community = %community_id,
            len = fixed_len,
            cap = PRESENCE_ROW_CAP,
            "presence row over its slot before the history ad; not written",
        );
        return;
    }
    presence.history_ranges_encrypted =
        fit_history_ad(deps, community_id, &presence, history_ranges);

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

    if presence_json.len() > PRESENCE_ROW_CAP {
        tracing::warn!(
            community = %community_id,
            len = presence_json.len(),
            cap = PRESENCE_ROW_CAP,
            "signed presence row over its slot; not written",
        );
        return;
    }

    let route_len = presence.route_blob.len();
    let target = RowTarget {
        community_id,
        registry_key,
        subkey: subkey_idx,
        writer: kp_str,
        my_pseudonym_hex,
    };
    match deps
        .write_presence_to_registry_subkey(registry_key, subkey_idx, presence_json.clone(), kp_str)
        .await
    {
        Ok(RowWrite::Stored) => tracing::debug!(
            community = %community_id,
            registry_key,
            subkey = subkey_idx,
            route_len,
            "presence row written",
        ),
        Ok(RowWrite::Superseded { seq, data }) => {
            resolve_superseded(deps, &target, presence_json, seq, &data).await;
        }
        Err(error) => tracing::warn!(
            community = %community_id,
            registry_key,
            subkey = subkey_idx,
            %error,
            "failed to write presence to registry",
        ),
    }
}

/// Where our row is written: enough to rewrite it after a supersede.
struct RowTarget<'a> {
    community_id: &'a str,
    registry_key: &'a str,
    subkey: u32,
    writer: &'a str,
    my_pseudonym_hex: &'a str,
}

/// A write to our slot came back superseded (plan C7.16). Veilid has
/// already adopted the network's value and stored it locally
/// (`set_value.rs:620-645`), so the classification decides:
///
/// - our own newer copy (a write the network holds and our local store
///   missed): write again now, and Veilid's local seq + 1 lands above it
///   (BEP 44's rejected-put-then-bump; libtorrent: "first retrieve it, then
///   modify it, then write it back");
/// - another member's validly signed row: a slot collision, possible only
///   because every member can derive every slot key from the shared seed
///   (ADR 0011 removes that). Never overwritten, which would start a write
///   war: reported;
/// - an unverifiable value: reported, not overwritten.
async fn resolve_superseded<D: CommunityPresenceDeps>(
    deps: &D,
    target: &RowTarget<'_>,
    presence_json: Vec<u8>,
    seq: Option<u32>,
    data: &[u8],
) {
    let RowTarget {
        community_id,
        registry_key,
        subkey,
        writer,
        my_pseudonym_hex,
    } = *target;
    match classify_superseding_row(data, my_pseudonym_hex) {
        SupersedingRow::Ours => {
            let rewrite = deps
                .write_presence_to_registry_subkey(registry_key, subkey, presence_json, writer)
                .await;
            match rewrite {
                Ok(RowWrite::Stored) => tracing::info!(
                    community = %community_id,
                    subkey,
                    superseding_seq = ?seq,
                    "presence row was behind our own newer copy on the network; rewritten above it",
                ),
                Ok(RowWrite::Superseded { seq: again, .. }) => tracing::warn!(
                    community = %community_id,
                    subkey,
                    superseding_seq = ?seq,
                    again_seq = ?again,
                    "presence row superseded again after the rewrite; the next tick writes",
                ),
                Err(error) => tracing::warn!(
                    community = %community_id,
                    subkey,
                    %error,
                    "presence row rewrite after a supersede failed",
                ),
            }
        }
        SupersedingRow::Member(author) => tracing::warn!(
            community = %community_id,
            subkey,
            superseding_seq = ?seq,
            author = %author,
            "another member's row holds our presence slot (slot collision); not overwritten",
        ),
        SupersedingRow::Unverified(reason) => tracing::warn!(
            community = %community_id,
            subkey,
            superseding_seq = ?seq,
            reason,
            "an unverified value holds our presence slot; not overwritten",
        ),
    }
}

/// The history ad that fits beside the row's fixed fields: the most
/// recently active channels first, as many as the slot holds. VeilidChat's
/// per-member status slot caps its vector the same way ("capped by encoded
/// size; least-recently-active authors are evicted first",
/// `veilidchat.proto` L242-247). E3.5 gives the ad its own subkey (`M/5`).
fn fit_history_ad<D: CommunityPresenceDeps>(
    deps: &D,
    community_id: &str,
    presence: &MemberPresence,
    mut ranges: Vec<HistoryRange>,
) -> Option<EncryptedHistoryRanges> {
    if ranges.is_empty() {
        return None;
    }
    ranges.sort_by(|a, b| b.newest_lamport.cmp(&a.newest_lamport));
    let mut probe = presence.clone();
    let mut fitted = None;
    let mut fitted_count = 0;
    for count in 1..=ranges.len() {
        // No current MEK: no ad at all this write.
        let encrypted =
            deps.encrypt_history_ranges_with_current_mek(community_id, &ranges[..count])?;
        probe.history_ranges_encrypted = Some(encrypted);
        if signed_len(&probe) > PRESENCE_ROW_CAP {
            break;
        }
        fitted = probe.history_ranges_encrypted.take();
        fitted_count = count;
    }
    if fitted_count < ranges.len() {
        tracing::debug!(
            community = %community_id,
            advertised = fitted_count,
            evicted = ranges.len() - fitted_count,
            "history ad capped by the row's slot (least recently active evicted)",
        );
    }
    fitted
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

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
