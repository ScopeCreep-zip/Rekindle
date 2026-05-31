//! Phase 18.d — origin pipeline.
//!
//! Ported from `src-tauri/src/services/community/create.rs::create_community`.
//! Creates a new community: derives the 255 slot pubkeys, creates the
//! three SMPL DHT records (governance + registry + #general channel),
//! writes the 5 genesis governance entries + creator presence, generates
//! the initial MEK, builds the in-memory `CommunityState`, kicks off
//! background services. Returns the governance record key as the
//! community ID.

use rand::RngCore;
use rekindle_governance::merge;
use rekindle_secrets::derive;
use rekindle_secrets::keys::{MediaEncryptionKey, SlotSeed};
use rekindle_types::governance::{GovernanceEntry, GovernanceSubkeyPayload};
use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
use rekindle_types::permissions;
use rekindle_types::presence::MemberPresence;

use crate::deps::{CommunityInsert, DiscoveredMember, GovernanceRuntimeDeps, MekSnapshot};
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;

const SLOTS_PER_SEGMENT: u32 = 255;
const CREATOR_SLOT: u32 = 0;
/// Owner role gets the all-ones RoleId so the on-the-wire role tag is
/// stable across community installs (architecture §Failure 4).
const OWNER_ROLE_ID: RoleId = RoleId([0xFFu8; 16]);

/// Generate a fresh random 16-byte channel id.
fn random_channel_id() -> ChannelId {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    ChannelId(bytes)
}

/// Map a `RoleId` to the legacy u32 form the src-tauri `RoleDefinition`
/// uses for `my_role_ids`. Mirrors `super::join::helpers::role_id_to_legacy_u32`
/// in src-tauri — first 4 LE bytes of the 16-byte RoleId.
fn role_id_to_legacy_u32(role_id: &RoleId) -> u32 {
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&role_id.0[..4]);
    u32::from_le_bytes(bytes)
}

/// Create a new community with flat SMPL governance (o_cnt:0 universal
/// schema). Returns the governance record key as the community ID.
pub async fn create_community<D: GovernanceRuntimeDeps>(
    deps: &D,
    name: &str,
) -> Result<String, GovernanceRuntimeError> {
    // 1. Generate the shared slot seed (32 bytes) — derives all 255 member
    //    slot keypairs deterministically.
    let slot_seed = SlotSeed::generate();

    // 2. Derive the 255 slot public keys for the universal SMPL schema.
    let mut member_pubkeys = Vec::with_capacity(SLOTS_PER_SEGMENT as usize);
    for i in 0..SLOTS_PER_SEGMENT {
        let sk = derive::derive_slot_keypair(&slot_seed.0, i).map_err(|e| {
            GovernanceRuntimeError::Crypto(format!(
                "slot keypair derivation failed at index {i}: {e}"
            ))
        })?;
        member_pubkeys.push(sk.verifying_key().to_bytes());
    }

    // 3-5. Create the three SMPL records.
    let gov_record = deps.create_smpl_record(&member_pubkeys).await?;
    let reg_record = deps.create_smpl_record(&member_pubkeys).await?;
    let ch_record = deps.create_smpl_record(&member_pubkeys).await?;
    let gov_key = gov_record.record_key.clone();
    let reg_key = reg_record.record_key.clone();
    let ch_key = ch_record.record_key.clone();

    // 6. Creator pseudonym from master secret + governance key.
    let master_secret = deps
        .identity_secret()
        .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?;
    let pseudonym_signing = derive::derive_community_pseudonym(&master_secret, &gov_key);
    let my_pseudo_bytes = pseudonym_signing.verifying_key().to_bytes();
    let my_pseudo_hex = hex::encode(my_pseudo_bytes);
    let my_pseudo = PseudonymKey(my_pseudo_bytes);

    // Creator's slot 0 keypair (writes to governance + registry under slot 0).
    let creator_slot_kp = derive::derive_slot_keypair(&slot_seed.0, CREATOR_SLOT).map_err(|e| {
        GovernanceRuntimeError::Crypto(format!("creator slot keypair derivation failed: {e}"))
    })?;
    let creator_writer = deps.format_writer_keypair(
        creator_slot_kp.verifying_key().to_bytes(),
        creator_slot_kp.to_bytes(),
    );

    // 7. Genesis governance entries (architecture §Failure 4 — five entries:
    //    community meta + @everyone + Owner role + RoleAssignment + #general).
    let channel_id = random_channel_id();
    let genesis_entries = vec![
        GovernanceEntry::CommunityMeta {
            name: Some(name.to_string()),
            description: None,
            icon_hash: None,
            banner_hash: None,
            lamport: 1,
        },
        GovernanceEntry::RoleDefinition {
            role_id: RoleId([0u8; 16]),
            name: "@everyone".into(),
            permissions: permissions::DEFAULT_EVERYONE,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 2,
        },
        GovernanceEntry::RoleDefinition {
            role_id: OWNER_ROLE_ID,
            name: "Owner".into(),
            permissions: permissions::ADMINISTRATOR,
            position: u32::MAX,
            color: 0xC1_7C_17,
            hoist: true,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 3,
        },
        GovernanceEntry::RoleAssignment {
            target: my_pseudo.clone(),
            role_id: OWNER_ROLE_ID,
            lamport: 4,
        },
        GovernanceEntry::ChannelCreated {
            channel_id,
            name: "general".into(),
            channel_type: "text".into(),
            record_key: ch_key.clone(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
            lamport: 5,
        },
    ];

    // Architecture §26 W26 — sign the genesis payload with the creator's
    // pseudonym secret so future readers can verify authorship.
    let mut genesis_payload = GovernanceSubkeyPayload {
        author_pseudonym: my_pseudo.clone(),
        entries: genesis_entries.clone(),
        signature: Vec::new(),
    };
    let genesis_sig =
        derive::sign_with_pseudonym(&pseudonym_signing, &genesis_payload.signing_bytes());
    genesis_payload.signature = genesis_sig.to_vec();
    let gov_payload = serde_json::to_vec(&genesis_payload)
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("genesis serialization failed: {e}")))?;
    let write_outcome = deps
        .set_dht_value(
            &gov_key,
            CREATOR_SLOT,
            gov_payload,
            Some(creator_writer.clone()),
        )
        .await?;
    if let Some(stale) = write_outcome {
        return Err(GovernanceRuntimeError::WriteConflict(stale.len()));
    }

    // 8. Write creator's MemberPresence to registry slot 0 — signed so
    //    peers can verify the creator authored this row.
    let mut presence = MemberPresence {
        pseudonym_key: my_pseudo.clone(),
        display_name: Some(deps.identity_display_name()),
        status: deps.identity_status().as_wire_str().into(),
        route_blob: deps.our_route_blob(),
        last_heartbeat: rekindle_utils::timestamp_secs(),
        ..Default::default()
    };
    let presence_sig =
        derive::sign_with_pseudonym(&pseudonym_signing, &presence.signing_bytes());
    presence.signature = presence_sig.to_vec();
    let presence_bytes = serde_json::to_vec(&presence)
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("presence serialization failed: {e}")))?;
    let reg_outcome = deps
        .set_dht_value(
            &reg_key,
            CREATOR_SLOT,
            presence_bytes,
            Some(creator_writer.clone()),
        )
        .await?;
    if let Some(stale) = reg_outcome {
        return Err(GovernanceRuntimeError::WriteConflict(stale.len()));
    }

    // 9. Generate the initial community MEK.
    let mek = MediaEncryptionKey::generate(1);
    let mek_generation = mek.generation();
    let mek_snapshot = MekSnapshot {
        generation: mek_generation,
        key_bytes: *mek.as_bytes(),
    };
    deps.insert_community_mek(&gov_key, mek_snapshot.clone());

    // 10. Merge genesis entries into a fresh GovernanceState — joiners
    //     re-merge from DHT reads; we precompute locally for the creator.
    let gov_state = merge::merge(&[(my_pseudo.clone(), genesis_entries)]);

    // 11. Build the CommunityInsert and hand it to the adapter to
    //     install into AppState.communities.
    let creator_role_ids = vec![0u32, role_id_to_legacy_u32(&OWNER_ROLE_ID)];
    let channel_id_hex = hex::encode(channel_id.0);
    let insert = CommunityInsert {
        id: gov_key.clone(),
        name: name.to_string(),
        channel_id_hex: channel_id_hex.clone(),
        channel_record_key: ch_key.clone(),
        governance_key: gov_key.clone(),
        registry_key: reg_key.clone(),
        registry_owner_keypair: reg_record.owner_keypair,
        dht_owner_keypair: gov_record.owner_keypair,
        slot_seed_hex: hex::encode(slot_seed.0),
        slot_keypair: creator_writer,
        my_pseudonym_hex: my_pseudo_hex.clone(),
        mek: mek_snapshot,
        governance_state: gov_state,
        lamport_counter: 5,
        creator_role_ids: creator_role_ids.clone(),
    };
    deps.insert_community(insert);

    // 12. Lost Cargo file cache + creator member row (so the Members panel
    //     populates immediately instead of waiting for the 30s presence
    //     poll — architecture §Failure 4).
    deps.ensure_files_cache_open(&gov_key);
    deps.persist_discovered_registry_members(
        &gov_key,
        vec![DiscoveredMember {
            segment_index: 0,
            slot_index: CREATOR_SLOT,
            presence: presence.clone(),
            role_ids: creator_role_ids,
        }],
    );

    // 13. Background services: watch + presence poll + DHT keepalive.
    deps.watch_community_records(&gov_key).await?;
    deps.spawn_presence_poll(&gov_key);
    deps.spawn_dht_keepalive(&gov_key);

    deps.emit_event(GovernanceRuntimeEvent::CommunityCreated {
        community_id: gov_key.clone(),
        name: name.to_string(),
    });

    Ok(gov_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_id_to_legacy_u32_owner() {
        // Owner role is all-0xFF → u32 first 4 LE bytes = 0xFFFFFFFF.
        assert_eq!(role_id_to_legacy_u32(&OWNER_ROLE_ID), u32::MAX);
    }

    #[test]
    fn role_id_to_legacy_u32_everyone() {
        // @everyone is all-zero → u32 = 0.
        assert_eq!(role_id_to_legacy_u32(&RoleId([0u8; 16])), 0);
    }

    #[test]
    fn role_id_to_legacy_u32_arbitrary_bytes() {
        let mut bytes = [0u8; 16];
        bytes[..4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        assert_eq!(role_id_to_legacy_u32(&RoleId(bytes)), 0xDEAD_BEEF);
    }

    #[test]
    fn random_channel_id_is_16_bytes_and_random() {
        let a = random_channel_id();
        let b = random_channel_id();
        assert_ne!(a.0, b.0, "two consecutive channel ids should differ");
    }

    #[test]
    fn constants_match_architecture_section_4_6() {
        assert_eq!(SLOTS_PER_SEGMENT, 255);
        assert_eq!(CREATOR_SLOT, 0);
        assert_eq!(OWNER_ROLE_ID.0, [0xFFu8; 16]);
    }
}
