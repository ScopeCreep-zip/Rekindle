//! Origin (plan E3.2, ADR 0011): create a community. The anchor `A`
//! (DFLT(1)) holds the `GenesisCert`, signed by `A`'s owner `K_A` (written
//! once, then released and zeroized) and by the creator pseudonym; the
//! creator's member record `M(p)` (DFLT(8)) is owned by a random `K_M(p)`
//! kept in the vault; the genesis ops are the creator's ops #0…n. The
//! registry and the `#general` channel record are still created until
//! E3.4/E3.3 delete them. The community id is `A`'s record key.

use rekindle_codec::governance_op::genesis;
use rekindle_crypto::keypair_text;
use rekindle_governance::genesis::genesis_signing_bytes;
use rekindle_governance::merge;
use rekindle_governance::state::GovernanceState;
use rekindle_records::lease::LeaseId;
use rekindle_records::schema;
use rekindle_secrets::derive;
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_secrets::keys::{MediaEncryptionKey, SlotSeed};
use rekindle_types::dht_layout::community_anchor;
use rekindle_types::governance::{AdmissionMode, GenesisCert, GovernanceEntry};
use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
use rekindle_types::permissions;
use rekindle_types::presence::MemberPresence;
use zeroize::Zeroizing;

use crate::deps::{
    CommunityInsert, DhtRecordInfo, DiscoveredMember, GovernanceRuntimeDeps, MekSnapshot,
};
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;
use crate::op_log;
use rekindle_records::lease::CommunityLeases;

use rekindle_types::dht_layout::SLOTS_PER_SEGMENT;

const CREATOR_SLOT: u32 = 0;
/// Owner role gets the all-ones RoleId so the on-the-wire role tag is
/// stable across community installs (architecture §Failure 4).
const OWNER_ROLE_ID: RoleId = RoleId([0xFFu8; 16]);
/// `@everyone`: the all-zero RoleId.
const EVERYONE_ROLE_ID: RoleId = RoleId([0u8; 16]);

/// Generate a fresh random 16-byte channel id from the host's generator.
fn random_channel_id(rng: &dyn rekindle_utils::RandomSource) -> ChannelId {
    ChannelId(rekindle_utils::random::id_bytes_16_from(rng))
}

/// Map a `RoleId` to the legacy u32 form the src-tauri `RoleDefinition`
/// uses for `my_role_ids`. Mirrors `super::join::helpers::role_id_to_legacy_u32`
/// in src-tauri — first 4 LE bytes of the 16-byte RoleId.
fn role_id_to_legacy_u32(role_id: &RoleId) -> u32 {
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&role_id.0[..4]);
    u32::from_le_bytes(bytes)
}

/// The genesis ops' bodies, the creator's ops #0…n in this order. The
/// admission mode is not among them: `GenesisCert.policy` pins it (plan
/// E3.2 decision (b)).
fn genesis_entries(
    name: &str,
    my_pseudo: &PseudonymKey,
    channel_id: ChannelId,
    channel_record_key: &str,
) -> Vec<GovernanceEntry> {
    vec![
        GovernanceEntry::CommunityMeta {
            name: Some(name.to_string()),
            description: None,
            icon_hash: None,
            banner_hash: None,
        },
        GovernanceEntry::RoleDefinition {
            role_id: EVERYONE_ROLE_ID,
            name: "@everyone".into(),
            permissions: permissions::DEFAULT_EVERYONE,
            position: 0,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
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
        },
        GovernanceEntry::RoleAssignment {
            target: my_pseudo.clone(),
            role_id: OWNER_ROLE_ID,
        },
        GovernanceEntry::ChannelCreated {
            channel_id,
            name: "general".into(),
            channel_type: "text".into(),
            record_key: channel_record_key.to_string(),
            category_id: None,
            position: 0,
            parent_voice_channel_id: None,
        },
    ]
}

/// The records origin creates; every lease is released on a failure.
struct OriginRecords {
    anchor: DhtRecordInfo,
    member: DhtRecordInfo,
    registry: DhtRecordInfo,
    channel: DhtRecordInfo,
}

/// Create a new community (plan E3.2, ADR 0011): the anchor `A` with its
/// `GenesisCert`, the creator's member record `M(p)`, the genesis ops, and
/// (until E3.4/E3.3 delete them) the registry and the `#general` record.
/// Returns `A`'s record key, the community id.
pub async fn create_community<D: GovernanceRuntimeDeps>(
    deps: &D,
    name: &str,
    admission: AdmissionMode,
) -> Result<String, GovernanceRuntimeError> {
    let slot_seed = SlotSeed::generate_with(deps.rng().as_ref());
    let mut member_pubkeys = Vec::with_capacity(SLOTS_PER_SEGMENT as usize);
    for i in 0..SLOTS_PER_SEGMENT {
        let sk = derive::derive_slot_keypair(&slot_seed.0, i).map_err(|e| {
            GovernanceRuntimeError::Crypto(format!(
                "slot keypair derivation failed at index {i}: {e}"
            ))
        })?;
        member_pubkeys.push(sk.verifying_key().to_bytes());
    }
    // A, M(p), the registry, #general; a failure releases what exists.
    let mut created: Vec<DhtRecordInfo> = Vec::with_capacity(4);
    for which in 0..4 {
        let result = match which {
            0 => deps.create_record(&schema::community_anchor()).await,
            1 => deps.create_record(&schema::member_record()).await,
            _ => deps.create_smpl_record(&member_pubkeys).await,
        };
        match result {
            Ok(record) => created.push(record),
            Err(e) => {
                for record in created {
                    deps.release_record(record.lease).await;
                }
                return Err(e);
            }
        }
    }
    let leases: Vec<LeaseId> = created.iter().map(|r| r.lease).collect();
    let mut it = created.into_iter();
    let (Some(anchor), Some(member), Some(registry), Some(channel)) =
        (it.next(), it.next(), it.next(), it.next())
    else {
        return Err(GovernanceRuntimeError::Adapter(
            "origin: a record is missing".into(),
        ));
    };
    let records = OriginRecords {
        anchor,
        member,
        registry,
        channel,
    };
    match seed_community(deps, name, admission, &slot_seed, records).await {
        Ok(anchor) => Ok(anchor),
        Err(e) => {
            for lease in leases {
                deps.release_record(lease).await;
            }
            Err(e)
        }
    }
}

/// Parse a created record's owner keypair text into its signing key.
fn owner_signing_key(
    info: &DhtRecordInfo,
    what: &str,
) -> Result<(Zeroizing<String>, SigningKey), GovernanceRuntimeError> {
    let text = Zeroizing::new(info.owner_keypair.clone().ok_or_else(|| {
        GovernanceRuntimeError::Adapter(format!("origin: the {what} has no owner keypair"))
    })?);
    let (_, secret) = keypair_text::parse_keypair_text(&text)
        .map_err(|e| GovernanceRuntimeError::Crypto(format!("{what} owner: {e}")))?;
    let secret = Zeroizing::new(secret);
    Ok((text, SigningKey::from_bytes(&secret)))
}

/// Sign and write the `GenesisCert` to `A`/0 with `K_A`, then release `A`
/// and drop `K_A` (zeroized; never persisted). Returns the cert and its
/// stored bytes.
async fn write_genesis<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &DhtRecordInfo,
    cert: GenesisCert,
    pseudonym: &SigningKey,
) -> Result<(GenesisCert, Vec<u8>), GovernanceRuntimeError> {
    let (k_a_text, k_a) = owner_signing_key(anchor, "anchor")?;
    let anchor_owner = k_a.verifying_key().to_bytes();
    let unsigned = genesis::genesis_unsigned_bytes(&cert, &anchor_owner);
    let message = genesis_signing_bytes(&anchor.record_key, &unsigned);
    let signed = genesis::SignedGenesis {
        anchor_sig: k_a.sign(&message).to_bytes(),
        creator_sig: pseudonym.sign(&message).to_bytes(),
        cert,
        anchor_owner,
    };
    drop(k_a);
    let bytes = genesis::encode_genesis(&signed)
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("genesis cert: {e}")))?;
    let outcome = deps
        .set_dht_value(
            anchor.lease,
            community_anchor::GENESIS_CERT,
            bytes.clone(),
            Some(k_a_text.to_string()),
        )
        .await;
    drop(k_a_text);
    deps.release_record(anchor.lease).await;
    if let Some(stale) = outcome? {
        return Err(GovernanceRuntimeError::WriteConflict(stale.len()));
    }
    Ok((signed.cert, bytes))
}

/// Origin after the records exist: the cert, the genesis ops, the
/// creator's registry row, the first MEK, the local install, and handing
/// the records to the host.
async fn seed_community<D: GovernanceRuntimeDeps>(
    deps: &D,
    name: &str,
    admission: AdmissionMode,
    slot_seed: &SlotSeed,
    records: OriginRecords,
) -> Result<String, GovernanceRuntimeError> {
    let OriginRecords {
        anchor,
        member,
        registry: reg_record,
        channel: ch_record,
    } = records;
    let anchor_key = anchor.record_key.clone();
    let reg_key = reg_record.record_key.clone();
    let ch_key = ch_record.record_key.clone();

    // The creator pseudonym from the master secret and the anchor.
    let master_secret = deps
        .identity_secret()
        .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?;
    let pseudonym_signing = derive::derive_community_pseudonym(&master_secret, &anchor_key);
    let my_pseudo_bytes = pseudonym_signing.verifying_key().to_bytes();
    let my_pseudo_hex = hex::encode(my_pseudo_bytes);
    let my_pseudo = PseudonymKey(my_pseudo_bytes);

    // M(p)'s owner K_M(p): the public half goes in the cert, the keypair
    // to the vault through the insert (before the community row, D1).
    let (k_m_text, k_m) = owner_signing_key(&member, "member record")?;
    let member_owner = keypair_text::public_key_text(&k_m.verifying_key().to_bytes());
    drop(k_m);
    let member_key = member.record_key.clone();
    deps.release_record(member.lease).await;

    let cert = GenesisCert {
        name: name.to_string(),
        policy: admission,
        initial_roles: vec![EVERYONE_ROLE_ID, OWNER_ROLE_ID],
        creator_pseudonym: my_pseudo.clone(),
        creator_member_record: member_key.clone(),
        creator_member_record_owner: member_owner,
        created_at: deps.clock().now_secs(),
    };
    let (cert, cert_bytes) = write_genesis(deps, &anchor, cert, &pseudonym_signing).await?;

    // The genesis ops: the creator's ops #0…n.
    let channel_id = random_channel_id(deps.rng().as_ref());
    let mut set = op_log::VerifiedSet::default();
    let mut genesis_ops = Vec::new();
    for body in genesis_entries(name, &my_pseudo, channel_id, &ch_key) {
        let (op, encoded) = op_log::build_op(
            &anchor_key,
            &cert,
            &GovernanceState::default(),
            &set,
            &pseudonym_signing,
            body,
            cert.created_at,
        )?;
        genesis_ops.push(op_log::held(
            &op,
            member_key.clone(),
            encoded,
            op_log::OpFate::Applied,
        ));
        set.ops.push(op);
    }
    let gov_state = merge::merge(&cert, &set.ops);

    let creator_slot_kp = derive::derive_slot_keypair(&slot_seed.0, CREATOR_SLOT).map_err(|e| {
        GovernanceRuntimeError::Crypto(format!("creator slot keypair derivation failed: {e}"))
    })?;
    let creator_writer = deps.format_writer_keypair(
        creator_slot_kp.verifying_key().to_bytes(),
        creator_slot_kp.to_bytes(),
    );

    // The creator's MemberPresence in registry slot 0, signed so peers can
    // verify the creator authored it (until E3.4 replaces the registry).
    let mut presence = MemberPresence {
        pseudonym_key: my_pseudo.clone(),
        display_name: Some(deps.identity_display_name()),
        status: deps.identity_status().as_wire_str().into(),
        route_blob: deps.our_route_blob(),
        last_heartbeat: deps.clock().now_secs(),
        ..Default::default()
    };
    let presence_sig = derive::sign_with_pseudonym(&pseudonym_signing, &presence.signing_bytes());
    presence.signature = presence_sig.to_vec();
    let presence_bytes = serde_json::to_vec(&presence).map_err(|e| {
        GovernanceRuntimeError::Encoding(format!("presence serialization failed: {e}"))
    })?;
    let reg_outcome = deps
        .set_dht_value(
            reg_record.lease,
            CREATOR_SLOT,
            presence_bytes,
            Some(creator_writer.clone()),
        )
        .await?;
    if let Some(stale) = reg_outcome {
        return Err(GovernanceRuntimeError::WriteConflict(stale.len()));
    }

    let mek = MediaEncryptionKey::generate_with(1, deps.rng().as_ref());
    let mek_snapshot = MekSnapshot {
        generation: mek.generation(),
        key_bytes: *mek.as_bytes(),
    };
    deps.insert_community_mek(&anchor_key, mek_snapshot.clone());

    let creator_role_ids = vec![0u32, role_id_to_legacy_u32(&OWNER_ROLE_ID)];
    let insert = CommunityInsert {
        id: anchor_key.clone(),
        name: name.to_string(),
        channel_id_hex: hex::encode(channel_id.0),
        channel_record_key: ch_key.clone(),
        registry_key: reg_key.clone(),
        registry_owner_keypair: reg_record.owner_keypair,
        slot_seed_hex: hex::encode(slot_seed.0),
        slot_keypair: creator_writer,
        my_pseudonym_hex: my_pseudo_hex.clone(),
        mek: mek_snapshot,
        governance_state: gov_state,
        creator_role_ids: creator_role_ids.clone(),
        genesis_cert: cert_bytes,
        member_record: member_key,
        member_record_owner_keypair: k_m_text,
    };
    deps.insert_community(insert);

    // The genesis ops into the op store (after the row they reference),
    // then onto the governance-op stream, which appends them to M(p).
    for op in genesis_ops {
        deps.store_governance_op(&anchor_key, &op).await?;
        deps.publish_governance_op(&anchor_key, op.encoded)?;
    }

    deps.ensure_files_cache_open(&anchor_key);
    deps.persist_discovered_registry_members(
        &anchor_key,
        vec![DiscoveredMember {
            segment_index: 0,
            slot_index: CREATOR_SLOT,
            presence: presence.clone(),
            role_ids: creator_role_ids,
        }],
    );

    // Nothing else writes the channel record until a first message.
    crate::records::publish_created(deps, ch_record.lease, &slot_seed.0, 0).await?;

    deps.community_records_ready(
        &anchor_key,
        CommunityLeases {
            registry: Some(reg_record.lease),
            channels: std::collections::HashMap::from([(channel_id, ch_record.lease)]),
            ..CommunityLeases::default()
        },
    )
    .await;

    deps.emit_event(GovernanceRuntimeEvent::CommunityCreated {
        community_id: anchor_key.clone(),
        name: name.to_string(),
    });

    Ok(anchor_key)
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
        let a = random_channel_id(&rekindle_utils::OsRandom);
        let b = random_channel_id(&rekindle_utils::OsRandom);
        assert_ne!(a.0, b.0, "two consecutive channel ids should differ");
    }

    #[test]
    fn constants_match_architecture_section_4_6() {
        assert_eq!(SLOTS_PER_SEGMENT, 255);
        assert_eq!(CREATOR_SLOT, 0);
        assert_eq!(OWNER_ROLE_ID.0, [0xFFu8; 16]);
    }
}

#[cfg(test)]
mod genesis_tests {
    /// No admission entry among the genesis ops: the cert pins the mode.
    #[test]
    fn genesis_ops_are_meta_roles_owner_and_general() {
        let entries = super::genesis_entries(
            "c",
            &rekindle_types::id::PseudonymKey([1u8; 32]),
            rekindle_types::id::ChannelId([2u8; 16]),
            "rec",
        );
        let kinds: Vec<&str> = entries
            .iter()
            .map(|e| match e {
                super::GovernanceEntry::CommunityMeta { .. } => "meta",
                super::GovernanceEntry::RoleDefinition { .. } => "role",
                super::GovernanceEntry::RoleAssignment { .. } => "assign",
                super::GovernanceEntry::ChannelCreated { .. } => "channel",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["meta", "role", "role", "assign", "channel"]);
    }
}
