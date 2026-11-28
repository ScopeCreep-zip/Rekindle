//! Phase 18.g — join entry-point + pure helpers.
//!
//! Ported from `src-tauri/src/services/community/join/{flow,helpers,bootstrap}.rs`.
//!
//! Chiral split: this module + `join_stages.rs` host the PROTOCOL
//! primitives (identity derivation, invite lookup, slot claim algorithm,
//! presence collection, governance snapshot). The src-tauri side keeps
//! the orchestrator that constructs `CommunityState` from these
//! primitives' outputs and spawns the background services. Matches the
//! Phase 17 mek_rotation pattern — protocol in crate, AppState
//! mutations in src-tauri.

use std::collections::{HashMap, HashSet};

use rekindle_secrets::derive;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::error::GovernanceRuntimeError;

/// Result of `derive_join_identity` — the joiner's pseudonym for a
/// specific community plus the signing key used to author governance +
/// presence writes under their own slot.
pub struct JoinIdentity {
    pub pseudo_hex: String,
    pub pseudo: PseudonymKey,
    pub pseudonym_signing: SigningKey,
}

/// Online-member snapshot used to populate `gossip.online_members` /
/// `gossip.peers` after a successful join. Adapter maps to the
/// src-tauri `OnlineMember` shape.
#[derive(Debug, Clone)]
pub struct JoinOnlineMember {
    pub route_blob: Vec<u8>,
    pub status: String,
    pub last_seen: u64,
}

/// Aggregate initial-presence state collected during join (architecture
/// §13.4 — DHT registry scan + bootstrap bundle hints).
#[derive(Debug, Clone, Default)]
pub struct InitialPresence {
    pub peers: HashMap<String, JoinOnlineMember>,
    pub online: HashMap<String, JoinOnlineMember>,
    pub known_members: HashSet<String>,
    /// Full W26-verified `MemberPresence` rows discovered in the cold-join
    /// registry scan, paired with the registry slot index they occupy. The
    /// src-tauri orchestrator turns these into durable `community_members`
    /// rows — the roster that `get_community_members` reads — while the
    /// `online`/`peers` maps above remain the *ephemeral* online overlay.
    /// Separating durable membership from ephemeral presence is why a fresh
    /// joiner can see peers at all: like Matrix's `m.room.member` room state
    /// (delivered eagerly + completely at join) vs. `m.presence` EDUs
    /// (architecture §13.4). Excludes the joiner's own slot (its row rides in
    /// `ClaimedSlot::self_presence`).
    pub discovered: Vec<(u32, rekindle_types::presence::MemberPresence)>,
}

/// Derive the joiner's pseudonym + signing key for a specific community.
/// Pure — takes the master `identity_secret` + the governance record key
/// and produces the community-scoped pseudonym via `derive_community_pseudonym`.
pub fn derive_join_identity(identity_secret: &[u8; 32], anchor_str: &str) -> JoinIdentity {
    let pseudonym_signing = derive::derive_community_pseudonym(identity_secret, anchor_str);
    let pseudo_bytes = pseudonym_signing.verifying_key().to_bytes();
    JoinIdentity {
        pseudo_hex: hex::encode(pseudo_bytes),
        pseudo: PseudonymKey(pseudo_bytes),
        pseudonym_signing,
    }
}

/// Fallback community name when no `CommunityMeta` entry is present in
/// the merged governance state (shows "Community " + first 8 hex chars
/// of the anchor key).
#[must_use]
pub fn default_community_name(anchor: &str) -> String {
    format!("Community {}", &anchor[..8.min(anchor.len())])
}

/// M10.3 — look up an invite by `code_hash` in the raw governance subkey
/// entries. Returns the invite-secrets DFLT record key (a pointer; the
/// joiner fetches + decrypts the blob via `fetch_invite_secrets`) and the
/// inviter's pseudonym (the writer of the subkey carrying the
/// `InviteCreated` entry). Reader-validates: rejects revoked + expired
/// invites.
///
/// The inviter pseudonym is propagated to slot-claim so the joiner-side
/// quota check (`invite_quota::check_active_invites_cap`) can run before
/// any slot write.
pub fn find_invite_in_entries(
    subkeys: &[(PseudonymKey, Vec<GovernanceEntry>)],
    code_hash: &str,
    now_secs: u64,
) -> Result<(String, PseudonymKey), GovernanceRuntimeError> {
    match inspect_invite_in_entries(subkeys, code_hash, now_secs) {
        InviteGovStatus::Active {
            inviter,
            secrets_record_key,
        } => Ok((secrets_record_key, inviter)),
        InviteGovStatus::Revoked => Err(GovernanceRuntimeError::Adapter(
            "invite has been revoked".into(),
        )),
        InviteGovStatus::Expired => {
            Err(GovernanceRuntimeError::Adapter("invite has expired".into()))
        }
        InviteGovStatus::NotFound => Err(GovernanceRuntimeError::Adapter(
            "invalid invite code — no matching invite found in governance".into(),
        )),
    }
}

/// Governance's view of an invite, looked up by `code_hash` across the raw
/// subkey entries. The join path uses this to *enforce* revocation/expiry and
/// to recover the inviter pseudonym (for the quota sanity check) plus the
/// secrets pointer — but only as a fallback: a link-borne invite carries its
/// own `secrets_record_key`, so a missing or unverifiable `InviteCreated`
/// (→ `NotFound`) no longer blocks acceptance.
pub enum InviteGovStatus {
    Active {
        inviter: PseudonymKey,
        secrets_record_key: String,
    },
    Revoked,
    Expired,
    NotFound,
}

/// Classify an invite from the raw governance subkey entries. Reader-validates:
/// `Revoked`/`Expired` are reported whenever governance shows them so the caller
/// can reject; `Active` carries the inviter + secrets pointer; `NotFound` means
/// no matching `InviteCreated` is visible (governance incomplete or the entry
/// was never written).
#[must_use]
pub fn inspect_invite_in_entries(
    subkeys: &[(PseudonymKey, Vec<GovernanceEntry>)],
    code_hash: &str,
    now_secs: u64,
) -> InviteGovStatus {
    let mut revoked_ids: HashSet<[u8; 16]> = HashSet::new();
    for (_, entries) in subkeys {
        for entry in entries {
            if let GovernanceEntry::InviteRevoked { invite_id, .. } = entry {
                revoked_ids.insert(*invite_id);
            }
        }
    }

    for (author, entries) in subkeys {
        for entry in entries {
            if let GovernanceEntry::InviteCreated {
                invite_id,
                code_hash: ch,
                secrets_record_key,
                expires_at,
                ..
            } = entry
            {
                if ch == code_hash {
                    if revoked_ids.contains(invite_id) {
                        return InviteGovStatus::Revoked;
                    }
                    if let Some(exp) = expires_at {
                        if now_secs > *exp {
                            return InviteGovStatus::Expired;
                        }
                    }
                    return InviteGovStatus::Active {
                        inviter: author.clone(),
                        secrets_record_key: secrets_record_key.clone(),
                    };
                }
            }
        }
    }
    InviteGovStatus::NotFound
}

/// Merge a signed `MemberPresence` row into the in-progress
/// `initial_peers` + `initial_online` maps. Offline / no-route members
/// are added to `known_members` only (for display) but never routed to.
pub fn merge_presence_entry(
    presence: &mut InitialPresence,
    pseudonym_key: &str,
    status: &str,
    route_blob: &[u8],
    last_seen: u64,
) {
    presence.known_members.insert(pseudonym_key.to_string());
    if status == "offline" {
        return;
    }
    let member = JoinOnlineMember {
        route_blob: route_blob.to_vec(),
        status: status.to_string(),
        last_seen,
    };
    // Liveness ≠ reachability. A non-offline, heartbeating member is ONLINE
    // even with no route allocated yet (routes land asynchronously and are
    // frequently empty on a fresh join) — gating online on the route blob is
    // what made two fresh members invisible to each other at join. The route
    // is a separate reachability fact: only members with a route enter
    // `peers` (the set we actually send bytes to). Mirrors the steady-poll
    // classifier (`rekindle-presence` scan_row).
    presence
        .online
        .insert(pseudonym_key.to_string(), member.clone());
    if !route_blob.is_empty() {
        presence.peers.insert(pseudonym_key.to_string(), member);
    }
}

#[cfg(test)]
#[path = "join_tests.rs"]
mod tests;
