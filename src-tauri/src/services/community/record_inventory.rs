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

use rekindle_protocol::dht::pool::keepalive::WarmRecord;

use crate::state::community::CommunityState;

/// The complete set of DHT record keys this community owns — the authoritative
/// inventory the spec calls `CommunityRecords` (§10). Consumed by keepalive
/// warming (§14.1) and login rehydration so every record shares one durability
/// path. Result is sorted + de-duped.
pub fn warmable_records(cs: &CommunityState) -> Vec<WarmRecord> {
    let mut records: Vec<WarmRecord> = warmable_record_keys(cs)
        .into_iter()
        .map(WarmRecord::mutable)
        .collect();
    // The latest checkpoint's directory pages and our own op pages beyond
    // its cut, sealed (plan E3.2 S7 record warming).
    records.extend(rekindle_node::host::community::sealed_pages::warm_sealed_pages(cs));
    records
}

/// The mutable records of [`warmable_records`], sorted and de-duped.
pub fn warmable_record_keys(cs: &CommunityState) -> Vec<String> {
    let mut keys = Vec::new();

    // Governance + member registry (top-level SMPL records).
    keys.push(cs.anchor.clone().unwrap_or_else(|| cs.id.clone()));
    if let Some(key) = cs.member_registry_key.clone() {
        keys.push(key);
    }

    if let Some(gov) = cs.governance_state.as_ref() {
        // Plate Gate (§15.4) expansion segments — segment-N governance +
        // registry.
        for seg in &gov.segments {
            keys.push(seg.anchor.clone());
            keys.push(seg.registry_key.clone());
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

    // Our own member record and our watch ring's (plan E3.2 S7).
    keys.extend(rekindle_node::host::adapters::governance::watch_ring::warm_member_records(cs));

    keys.sort();
    keys.dedup();
    keys
}

#[cfg(test)]
#[path = "record_inventory_tests.rs"]
mod tests;
