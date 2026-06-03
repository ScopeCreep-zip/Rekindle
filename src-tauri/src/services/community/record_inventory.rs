//! The single authoritative per-community DHT-record inventory.
//!
//! Architecture §10 ("Open once, keep open", architecture-v2.md:695-721)
//! designates `CommunityRecords` as the one inventory that every lifecycle
//! mechanism consumes — periodic warming (§14.1 Mutual Aid), login
//! rehydration, open+track on join/login, and leave teardown. Before this
//! module, keepalive reassembled its own partial key list inline, so each
//! new record type had to be hand-added and was silently forgotten — the
//! orphan-record bug class (a record created but never registered cannot be
//! kept alive).
//!
//! `warmable_record_keys` is that single assembly point: a strict superset of
//! the old inline list plus the two previously-orphaned record classes
//! (GovernanceOverflow spill pages, and this identity's own live
//! invite-secrets DFLT records). Adding a future record type means extending
//! this one function, nowhere else.

use crate::state::community::CommunityState;

/// The complete set of DHT record keys this community owns — the authoritative
/// inventory the spec calls `CommunityRecords` (§10). Consumed by keepalive
/// warming (§14.1) and login rehydration so every record shares one durability
/// path. Result is sorted + de-duped.
pub fn warmable_record_keys(cs: &CommunityState) -> Vec<String> {
    let mut keys = Vec::new();

    // Governance + member registry (top-level SMPL records).
    keys.push(cs.governance_key.clone().unwrap_or_else(|| cs.id.clone()));
    if let Some(key) = cs.member_registry_key.clone() {
        keys.push(key);
    }

    // Channel DHTLog records.
    keys.extend(cs.channel_log_keys.values().cloned());

    if let Some(gov) = cs.governance_state.as_ref() {
        // Plate Gate (§15.4) expansion segments — segment-N governance +
        // registry, and channel-segment records.
        for seg in &gov.segments {
            keys.push(seg.governance_key.clone());
            keys.push(seg.registry_key.clone());
        }
        for csr in gov.channel_segment_records.values() {
            keys.push(csr.record_key.clone());
        }

        // §14.1 — the inviter keeps its own live invite-secrets DFLT records
        // warm for the whole session (previously login-rehydrated only). Same
        // scan as `list_my_active_invite_secret_keys`: only invites we authored
        // (local-store hits), non-expired, with a non-empty record key.
        if let Some(my_pk_hex) = &cs.my_pseudonym_key {
            let now = rekindle_utils::timestamp_secs();
            for invite in gov.invites.values() {
                if hex::encode(invite.creator_pseudonym.0) != *my_pk_hex {
                    continue;
                }
                if invite.expires_at.is_some_and(|exp| exp <= now) {
                    continue;
                }
                if !invite.secrets_record_key.is_empty() {
                    keys.push(invite.secrets_record_key.clone());
                }
            }
        }
    }

    // GovernanceOverflow spill pages — the orphan that caused the reported
    // "key not found" join failure. Holds the author's own spill plus any
    // overflow chain this node followed as a reader (Mutual Aid: readers keep
    // what they read alive).
    keys.extend(
        cs.open_community_records
            .governance_overflow_keys
            .iter()
            .cloned(),
    );

    keys.sort();
    keys.dedup();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::community::CommunityRecords;
    use rekindle_governance::state::{GovernanceState, InviteState};
    use rekindle_types::id::PseudonymKey;
    use std::collections::{HashMap, HashSet, VecDeque};

    fn invite(creator: PseudonymKey, secrets_key: &str, expires_at: Option<u64>) -> InviteState {
        InviteState {
            code_hash: String::new(),
            max_uses: 1,
            expires_at,
            secrets_record_key: secrets_key.to_string(),
            created_lamport: 0,
            creator_pseudonym: creator,
        }
    }

    /// Minimal `CommunityState` for inventory assertions — every field set to an
    /// empty/None default except the record-key fields the inventory reads.
    fn community(
        my_pk: &PseudonymKey,
        gov: Option<GovernanceState>,
        overflow_keys: Vec<String>,
    ) -> CommunityState {
        let mut channel_log_keys = HashMap::new();
        channel_log_keys.insert("c1".to_string(), "chan-key-1".to_string());
        CommunityState {
            id: "gov-id".to_string(),
            name: "T".to_string(),
            description: None,
            channels: Vec::new(),
            categories: Vec::new(),
            my_role_ids: Vec::new(),
            roles: Vec::new(),
            dht_owner_keypair: None,
            my_pseudonym_key: Some(hex::encode(my_pk.0)),
            mek_generation: 0,
            member_registry_key: Some("reg-key".to_string()),
            my_subkey_index: None,
            my_segment_index: None,
            governance_key: Some("gov-key".to_string()),
            governance_state: gov,
            lamport_counter: 0,
            gossip: None,
            slot_keypair: None,
            channel_log_keys,
            registry_owner_keypair: None,
            slot_seed: None,
            known_members: HashSet::new(),
            member_roles: HashMap::new(),
            channel_sequences: HashMap::new(),
            pending_syncs: HashMap::new(),
            watched_records: HashSet::new(),
            record_sequences: HashMap::new(),
            peer_sequences: HashMap::new(),
            channel_last_send_at: HashMap::new(),
            peer_reliability: HashMap::new(),
            presence_poll_shutdown_tx: None,
            dht_keepalive_shutdown_tx: None,
            open_community_records: CommunityRecords {
                governance_overflow_keys: overflow_keys,
                ..CommunityRecords::default()
            },
            my_event_rsvps: HashMap::new(),
            event_rsvps_by_event: HashMap::new(),
            onboarding_complete: false,
            my_bio: None,
            my_pronouns: None,
            my_theme_color: None,
            my_badges: Vec::new(),
            my_avatar_ref: None,
            my_banner_ref: None,
            icon_hash: None,
            banner_hash: None,
            member_profiles: HashMap::new(),
            recent_member_joins: VecDeque::new(),
        }
    }

    #[test]
    fn includes_core_overflow_and_my_invites_excludes_foreign_invites() {
        let me = PseudonymKey([7u8; 32]);
        let other = PseudonymKey([9u8; 32]);
        let mut gov = GovernanceState::default();
        gov.invites
            .insert([1u8; 16], invite(me.clone(), "my-secrets", None));
        gov.invites
            .insert([2u8; 16], invite(other, "other-secrets", None));
        // An expired invite of mine must NOT be warmed.
        gov.invites
            .insert([3u8; 16], invite(me.clone(), "my-expired", Some(1)));

        let cs = community(&me, Some(gov), vec!["ovl-key".to_string()]);
        let keys = warmable_record_keys(&cs);

        assert!(keys.contains(&"gov-key".to_string()), "governance key");
        assert!(keys.contains(&"reg-key".to_string()), "registry key");
        assert!(keys.contains(&"chan-key-1".to_string()), "channel key");
        assert!(keys.contains(&"ovl-key".to_string()), "overflow key");
        assert!(keys.contains(&"my-secrets".to_string()), "my active invite");
        assert!(
            !keys.contains(&"other-secrets".to_string()),
            "another author's invite must be excluded"
        );
        assert!(
            !keys.contains(&"my-expired".to_string()),
            "my expired invite must be excluded"
        );
    }

    #[test]
    fn falls_back_to_community_id_when_no_governance_key() {
        let me = PseudonymKey([7u8; 32]);
        let mut cs = community(&me, None, Vec::new());
        cs.governance_key = None;
        let keys = warmable_record_keys(&cs);
        assert!(
            keys.contains(&"gov-id".to_string()),
            "must fall back to community id as the governance record key"
        );
    }

    #[test]
    fn result_is_sorted_and_deduped() {
        let me = PseudonymKey([7u8; 32]);
        // Overflow key duplicated with the channel key to prove dedup.
        let cs = community(
            &me,
            Some(GovernanceState::default()),
            vec!["chan-key-1".to_string(), "ovl-a".to_string()],
        );
        let keys = warmable_record_keys(&cs);
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted, "keys must be sorted");
        let mut deduped = keys.clone();
        deduped.dedup();
        assert_eq!(keys, deduped, "keys must be deduped");
    }
}
