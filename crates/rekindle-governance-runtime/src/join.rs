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
}

/// Derive the joiner's pseudonym + signing key for a specific community.
/// Pure — takes the master `identity_secret` + the governance record key
/// and produces the community-scoped pseudonym via `derive_community_pseudonym`.
pub fn derive_join_identity(identity_secret: &[u8; 32], governance_key_str: &str) -> JoinIdentity {
    let pseudonym_signing = derive::derive_community_pseudonym(identity_secret, governance_key_str);
    let pseudo_bytes = pseudonym_signing.verifying_key().to_bytes();
    JoinIdentity {
        pseudo_hex: hex::encode(pseudo_bytes),
        pseudo: PseudonymKey(pseudo_bytes),
        pseudonym_signing,
    }
}

/// Fallback community name when no `CommunityMeta` entry is present in
/// the merged governance state (shows "Community " + first 8 hex chars
/// of the governance key).
#[must_use]
pub fn default_community_name(governance_key: &str) -> String {
    format!(
        "Community {}",
        &governance_key[..8.min(governance_key.len())]
    )
}

/// M10.3 — look up an invite by `code_hash` in the raw governance subkey
/// entries. Returns the encrypted secrets blob and the inviter's
/// pseudonym (the writer of the subkey carrying the `InviteCreated`
/// entry). Reader-validates: rejects revoked + expired invites.
///
/// The inviter pseudonym is propagated to slot-claim so the joiner-side
/// quota check (`invite_quota::check_active_invites_cap`) can run before
/// any slot write.
pub fn find_invite_in_entries(
    subkeys: &[(PseudonymKey, Vec<GovernanceEntry>)],
    code_hash: &str,
) -> Result<(String, PseudonymKey), GovernanceRuntimeError> {
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
                encrypted_secrets,
                expires_at,
                ..
            } = entry
            {
                if ch == code_hash {
                    if revoked_ids.contains(invite_id) {
                        return Err(GovernanceRuntimeError::Adapter(
                            "invite has been revoked".into(),
                        ));
                    }
                    if let Some(exp) = expires_at {
                        if rekindle_utils::timestamp_secs() > *exp {
                            return Err(GovernanceRuntimeError::Adapter(
                                "invite has expired".into(),
                            ));
                        }
                    }
                    return Ok((encrypted_secrets.clone(), author.clone()));
                }
            }
        }
    }
    Err(GovernanceRuntimeError::Adapter(
        "invalid invite code — no matching invite found in governance".into(),
    ))
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
    if status == "offline" || route_blob.is_empty() {
        return;
    }
    let member = JoinOnlineMember {
        route_blob: route_blob.to_vec(),
        status: status.to_string(),
        last_seen,
    };
    presence
        .peers
        .insert(pseudonym_key.to_string(), member.clone());
    presence.online.insert(pseudonym_key.to_string(), member);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_community_name_short_governance_key() {
        assert_eq!(default_community_name("abc"), "Community abc");
    }

    #[test]
    fn default_community_name_truncates_to_eight() {
        assert_eq!(
            default_community_name("abcdefghijklmnop"),
            "Community abcdefgh"
        );
    }

    #[test]
    fn derive_join_identity_is_deterministic() {
        let secret = [1u8; 32];
        let gov_key = "test-community-key";
        let a = derive_join_identity(&secret, gov_key);
        let b = derive_join_identity(&secret, gov_key);
        assert_eq!(a.pseudo_hex, b.pseudo_hex);
        assert_eq!(a.pseudo.0, b.pseudo.0);
    }

    #[test]
    fn derive_join_identity_differs_by_community() {
        let secret = [1u8; 32];
        let a = derive_join_identity(&secret, "community-a");
        let b = derive_join_identity(&secret, "community-b");
        assert_ne!(a.pseudo_hex, b.pseudo_hex);
    }

    #[test]
    fn merge_presence_offline_only_records_known() {
        let mut presence = InitialPresence::default();
        merge_presence_entry(&mut presence, "abc", "offline", &[1, 2, 3], 100);
        assert!(presence.known_members.contains("abc"));
        assert!(presence.peers.is_empty());
        assert!(presence.online.is_empty());
    }

    #[test]
    fn merge_presence_empty_route_skips_routing() {
        let mut presence = InitialPresence::default();
        merge_presence_entry(&mut presence, "abc", "online", &[], 100);
        assert!(presence.known_members.contains("abc"));
        assert!(presence.peers.is_empty());
        assert!(presence.online.is_empty());
    }

    #[test]
    fn merge_presence_online_with_route_populates_all() {
        let mut presence = InitialPresence::default();
        merge_presence_entry(&mut presence, "abc", "online", &[7, 8, 9], 42);
        assert!(presence.known_members.contains("abc"));
        assert_eq!(presence.peers.len(), 1);
        assert_eq!(presence.online.len(), 1);
        assert_eq!(presence.peers["abc"].route_blob, vec![7, 8, 9]);
        assert_eq!(presence.online["abc"].last_seen, 42);
    }

    #[test]
    fn find_invite_returns_inviter_pseudonym() {
        let author = PseudonymKey([7u8; 32]);
        let entries = vec![(
            author.clone(),
            vec![GovernanceEntry::InviteCreated {
                invite_id: [1u8; 16],
                code_hash: "abc".into(),
                max_uses: 0,
                expires_at: None,
                encrypted_secrets: "secret-blob".into(),
                lamport: 1,
            }],
        )];
        let (blob, inviter) = find_invite_in_entries(&entries, "abc").expect("found");
        assert_eq!(blob, "secret-blob");
        assert_eq!(inviter.0, author.0);
    }

    #[test]
    fn find_invite_rejects_revoked() {
        let author = PseudonymKey([7u8; 32]);
        let entries = vec![(
            author.clone(),
            vec![
                GovernanceEntry::InviteCreated {
                    invite_id: [1u8; 16],
                    code_hash: "abc".into(),
                    max_uses: 0,
                    expires_at: None,
                    encrypted_secrets: "x".into(),
                    lamport: 1,
                },
                GovernanceEntry::InviteRevoked {
                    invite_id: [1u8; 16],
                    lamport: 2,
                },
            ],
        )];
        let err = find_invite_in_entries(&entries, "abc").expect_err("revoked");
        match err {
            GovernanceRuntimeError::Adapter(msg) => assert!(msg.contains("revoked")),
            other => panic!("expected Adapter(revoked), got {other:?}"),
        }
    }

    #[test]
    fn find_invite_no_match_errors() {
        let entries = vec![];
        let err = find_invite_in_entries(&entries, "nope").expect_err("no match");
        match err {
            GovernanceRuntimeError::Adapter(msg) => assert!(msg.contains("no matching invite")),
            other => panic!("expected Adapter(no matching invite), got {other:?}"),
        }
    }
}
