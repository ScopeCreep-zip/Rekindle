use std::sync::Arc;

use rekindle_governance::merge;
use rekindle_records::schema;
use rekindle_secrets::derive;
use rekindle_secrets::keys::{MediaEncryptionKey, SlotSeed};
use rekindle_types::governance::{GovernanceEntry, GovernanceSubkeyPayload};
use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
use rekindle_types::permissions;
use rekindle_types::presence::MemberPresence;
use veilid_core::{SetDHTValueOptions, CRYPTO_KIND_VLD0};

use crate::state::{AppState, ChannelInfo, ChannelType, CommunityState, GossipOverlay, RoleDefinition};
use crate::state_helpers;

/// Create a new community with flat SMPL governance (o_cnt:0).
///
/// Creates three SMPL DHT records (governance, registry, #general channel),
/// writes genesis governance entries, and starts background services.
/// Returns the governance record key as the community identifier.
pub async fn create_community(
    state: &Arc<AppState>,
    name: &str,
) -> Result<String, String> {
    let rc = state_helpers::routing_context(state)
        .ok_or("Veilid node not attached — cannot create community")?;

    // 1. Generate slot seed — shared secret for all member slot keypairs
    let slot_seed = SlotSeed::generate();

    // 2. Derive 255 slot public keys for the universal SMPL schema
    let mut member_pubkeys = Vec::with_capacity(255);
    for i in 0..255u32 {
        let sk = derive::derive_slot_keypair(&slot_seed.0, i)
            .map_err(|e| format!("slot keypair derivation failed at index {i}: {e}"))?;
        member_pubkeys.push(sk.verifying_key().to_bytes());
    }

    // 3. Create SMPL governance record (o_cnt:0, 255 member slots)
    let gov_schema = schema::community_smpl_schema(&member_pubkeys)
        .map_err(|e| format!("governance schema creation failed: {e}"))?;
    let gov_desc = rc
        .create_dht_record(CRYPTO_KIND_VLD0, gov_schema, None)
        .await
        .map_err(|e| format!("governance record creation failed: {e}"))?;
    let gov_typed_key = gov_desc.key().clone();
    let gov_key = gov_typed_key.to_string();
    // Owner keypair retained for reopening — NOT for writing (o_cnt:0).
    let gov_owner_keypair = gov_desc
        .owner_secret()
        .map(|s| veilid_core::KeyPair::new_from_parts(gov_desc.owner().clone(), s.value()));

    // 4. Create SMPL registry record (same universal schema)
    let reg_schema = schema::community_smpl_schema(&member_pubkeys)
        .map_err(|e| format!("registry schema creation failed: {e}"))?;
    let reg_desc = rc
        .create_dht_record(CRYPTO_KIND_VLD0, reg_schema, None)
        .await
        .map_err(|e| format!("registry record creation failed: {e}"))?;
    let reg_typed_key = reg_desc.key().clone();
    let reg_key = reg_typed_key.to_string();
    let reg_owner_keypair = reg_desc
        .owner_secret()
        .map(|s| veilid_core::KeyPair::new_from_parts(reg_desc.owner().clone(), s.value()));

    // 5. Create SMPL channel record for #general
    let ch_schema = schema::community_smpl_schema(&member_pubkeys)
        .map_err(|e| format!("channel schema creation failed: {e}"))?;
    let ch_desc = rc
        .create_dht_record(CRYPTO_KIND_VLD0, ch_schema, None)
        .await
        .map_err(|e| format!("channel record creation failed: {e}"))?;
    let ch_key = ch_desc.key().to_string();

    // 6. Derive creator's pseudonym from master secret + governance key
    let master_secret = {
        let guard = state.identity_secret.lock();
        *guard.as_ref().ok_or("identity secret not available")?
    };
    let pseudonym_signing = derive::derive_community_pseudonym(&master_secret, &gov_key);
    let my_pseudo_bytes = pseudonym_signing.verifying_key().to_bytes();
    let my_pseudo_hex = hex::encode(my_pseudo_bytes);
    let my_pseudo = PseudonymKey(my_pseudo_bytes);

    // Creator gets slot 0
    let creator_slot: u32 = 0;
    let creator_slot_kp = derive::derive_slot_keypair(&slot_seed.0, creator_slot)
        .map_err(|e| format!("creator slot keypair derivation failed: {e}"))?;
    let creator_slot_veilid = slot_signing_to_veilid(&creator_slot_kp);

    // 7. Write genesis governance entries to slot 0
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
            lamport: 2,
        },
        GovernanceEntry::ChannelCreated {
            channel_id,
            name: "general".into(),
            channel_type: "text".into(),
            record_key: ch_key.clone(),
            category_id: None,
            position: 0,
            lamport: 3,
        },
    ];
    let gov_payload = serde_json::to_vec(&GovernanceSubkeyPayload {
        author_pseudonym: my_pseudo.clone(),
        entries: genesis_entries.clone(),
    })
        .map_err(|e| format!("genesis serialization failed: {e}"))?;
    let write_opts = SetDHTValueOptions {
        writer: Some(creator_slot_veilid.clone()),
        ..Default::default()
    };
    rc.set_dht_value(gov_typed_key, creator_slot, gov_payload, Some(write_opts))
        .await
        .map_err(|e| format!("genesis governance write failed: {e}"))?;

    // 8. Write creator's MemberPresence to registry slot 0
    let presence = MemberPresence {
        pseudonym_key: my_pseudo.clone(),
        display_name: Some(state_helpers::identity_display_name(state)),
        status: "online".into(),
        route_blob: state_helpers::our_route_blob(state).unwrap_or_default(),
        last_heartbeat: rekindle_utils::timestamp_secs(),
        ..Default::default()
    };
    let presence_bytes = serde_json::to_vec(&presence)
        .map_err(|e| format!("presence serialization failed: {e}"))?;
    let reg_write_opts = SetDHTValueOptions {
        writer: Some(creator_slot_veilid.clone()),
        ..Default::default()
    };
    rc.set_dht_value(reg_typed_key, creator_slot, presence_bytes, Some(reg_write_opts))
        .await
        .map_err(|e| format!("registry presence write failed: {e}"))?;

    // 9. Generate initial MEK
    let mek = MediaEncryptionKey::generate(1);
    let mek_generation = mek.generation();
    state.mek_cache.lock().insert(
        gov_key.clone(),
        rekindle_crypto::group::media_key::MediaEncryptionKey::from_bytes(
            *mek.as_bytes(),
            mek_generation,
        ),
    );

    // 10. Build GovernanceState via CRDT merge
    let gov_state = merge::merge(&[(my_pseudo.clone(), genesis_entries)]);

    // 11. Build CommunityState
    let channel_id_hex = hex::encode(channel_id.0);
    let community = CommunityState {
        id: gov_key.clone(),
        name: name.to_string(),
        description: None,
        channels: vec![ChannelInfo {
            id: channel_id_hex.clone(),
            name: "general".to_string(),
            channel_type: ChannelType::Text,
            unread_count: 0,
            category_id: None,
            topic: String::new(),
            slowmode_seconds: None,
            nsfw: false,
            message_record_key: Some(ch_key.clone()),
            mek_generation: 0,
        }],
        categories: Vec::new(),
        my_role_ids: vec![0],
        roles: vec![RoleDefinition {
            id: 0,
            name: "@everyone".into(),
            color: 0,
            permissions: permissions::DEFAULT_EVERYONE,
            position: 0,
            hoist: false,
            mentionable: false,
        }],
        my_role: Some("owner".to_string()),
        dht_record_key: Some(gov_key.clone()),
        dht_owner_keypair: gov_owner_keypair.as_ref().map(std::string::ToString::to_string),
        my_pseudonym_key: Some(my_pseudo_hex.clone()),
        mek_generation,
        manifest_key: None,
        member_registry_key: Some(reg_key.clone()),
        my_subkey_index: Some(creator_slot),
        coordinator_pseudonym: None,
        coordinator_route_blob: None,
        coordinator_epoch: 0,
        governance_key: Some(gov_key.clone()),
        governance_state: Some(gov_state),
        lamport_counter: 3,
        gossip: Some(GossipOverlay::default()),
        slot_keypair: Some(creator_slot_veilid.to_string()),
        manifest_owner_keypair: None,
        channel_log_keys: [(channel_id_hex, ch_key.clone())].into_iter().collect(),
        channel_sequences: std::collections::HashMap::new(),
        pending_syncs: std::collections::HashMap::new(),
        peer_sequences: std::collections::HashMap::new(),
        registry_owner_keypair: reg_owner_keypair.as_ref().map(std::string::ToString::to_string),
        slot_seed: Some(hex::encode(slot_seed.0)),
        member_roles: std::collections::HashMap::new(),
        known_members: [my_pseudo_hex].into_iter().collect(),
        presence_poll_shutdown_tx: None,
        dht_keepalive_shutdown_tx: None,
        open_community_records: crate::state::CommunityRecords::default(),
    };

    state.communities.write().insert(gov_key.clone(), community);

    // Track opened records
    {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(&gov_key) {
            cs.open_community_records.manifest_key = Some(gov_key.clone());
            cs.open_community_records.registry_key = Some(reg_key.clone());
            cs.open_community_records.registry_writer =
                reg_owner_keypair.as_ref().map(std::string::ToString::to_string);
            cs.open_community_records.channel_keys = vec![ch_key.clone()];
            cs.open_community_records.records_open = true;
        }
    }

    // 12. Start background services (no coordinator handle)
    super::presence::start_presence_poll(state.clone(), gov_key.clone());
    super::keepalive::start_dht_keepalive(state.clone(), gov_key.clone());

    tracing::info!(name = %name, governance_key = %gov_key, "community created with flat SMPL governance");
    Ok(gov_key)
}

/// Convert an Ed25519 SigningKey to a Veilid KeyPair for DHT writes.
pub(crate) fn slot_signing_to_veilid(sk: &rekindle_secrets::ed25519_dalek::SigningKey) -> veilid_core::KeyPair {
    let pub_bytes = sk.verifying_key().to_bytes();
    let secret_bytes = sk.to_bytes();
    let bare_pub = veilid_core::BarePublicKey::new(&pub_bytes);
    let bare_secret = veilid_core::BareSecretKey::new(&secret_bytes);
    let pubkey = veilid_core::PublicKey::new(CRYPTO_KIND_VLD0, bare_pub);
    veilid_core::KeyPair::new_from_parts(pubkey, bare_secret)
}

/// Generate a random 16-byte channel ID.
fn random_channel_id() -> ChannelId {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    ChannelId(bytes)
}

