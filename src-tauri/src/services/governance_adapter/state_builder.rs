//! Phase 18.h — `CommunityState` builder.
//!
//! Extracts the heaviest method (`insert_community`) from `deps_impl.rs`:
//! given a `CommunityInsert` DTO from the crate-side `origin` flow,
//! constructs a fresh `CommunityState` and inserts it into AppState
//! together with the community MEK. ~100 LoC of straight-line field
//! initialization.

use std::collections::HashMap;
use std::sync::Arc;

use rekindle_crypto::group::media_key::MediaEncryptionKey as CryptoMek;
use rekindle_governance_runtime::CommunityInsert;

use crate::state::{
    AppState, ChannelInfo, ChannelType, CommunityRecords, CommunityState, GossipOverlay,
    RoleDefinition,
};

pub(super) fn insert_community_into_state(state: &Arc<AppState>, community: CommunityInsert) {
    let CommunityInsert {
        id,
        name,
        channel_id_hex,
        channel_record_key,
        governance_key,
        registry_key,
        registry_owner_keypair,
        dht_owner_keypair,
        slot_seed_hex,
        slot_keypair,
        my_pseudonym_hex,
        mek,
        governance_state,
        lamport_counter,
        creator_role_ids,
    } = community;

    let channels = vec![ChannelInfo {
        id: channel_id_hex.clone(),
        name: "general".to_string(),
        channel_type: ChannelType::Text,
        unread_count: 0,
        category_id: None,
        topic: String::new(),
        forum_tags: None,
        stage_speakers: Vec::new(),
        stage_moderator: None,
        slowmode_seconds: None,
        nsfw: false,
        message_record_key: Some(channel_record_key.clone()),
        mek_generation: 0,
        notification_level: "all".to_string(),
        notification_sound_ref: None,
        parent_voice_channel_id: None,
    }];

    let roles: Vec<RoleDefinition> = governance_state
        .roles
        .iter()
        .map(|(role_id, role)| RoleDefinition {
            id: u32::from_le_bytes([role_id.0[0], role_id.0[1], role_id.0[2], role_id.0[3]]),
            name: role.name.clone(),
            color: role.color,
            permissions: role.permissions,
            position: role.position.cast_signed(),
            hoist: role.hoist,
            mentionable: role.mentionable,
            self_assignable: role.self_assignable,
            exclusion_group: None,
        })
        .collect();

    let open_records = CommunityRecords {
        governance_key: Some(governance_key.clone()),
        registry_key: Some(registry_key.clone()),
        registry_writer: registry_owner_keypair.clone(),
        channel_keys: vec![channel_record_key.clone()],
        records_open: true,
        ..Default::default()
    };

    let cs = CommunityState {
        id: id.clone(),
        name,
        description: None,
        icon_hash: None,
        banner_hash: None,
        channels,
        categories: Vec::new(),
        my_role_ids: creator_role_ids,
        roles,
        dht_owner_keypair,
        my_pseudonym_key: Some(my_pseudonym_hex.clone()),
        mek_generation: mek.generation,
        member_registry_key: Some(registry_key),
        my_subkey_index: Some(0),
        my_segment_index: Some(0),
        governance_key: Some(governance_key),
        governance_state: Some(governance_state),
        lamport_counter,
        gossip: Some(GossipOverlay::default()),
        slot_keypair: Some(slot_keypair),
        channel_log_keys: [(channel_id_hex, channel_record_key)]
            .into_iter()
            .collect(),
        channel_sequences: HashMap::new(),
        pending_syncs: HashMap::new(),
        watched_records: std::collections::HashSet::new(),
        record_sequences: HashMap::new(),
        peer_sequences: HashMap::new(),
        channel_last_send_at: HashMap::new(),
        peer_reliability: HashMap::new(),
        registry_owner_keypair,
        slot_seed: Some(slot_seed_hex),
        member_roles: HashMap::new(),
        known_members: std::iter::once(my_pseudonym_hex.clone()).collect(),
        presence_poll_shutdown_tx: None,
        dht_keepalive_shutdown_tx: None,
        open_community_records: open_records,
        my_event_rsvps: HashMap::new(),
        event_rsvps_by_event: HashMap::new(),
        onboarding_complete: true,
        my_bio: None,
        my_pronouns: None,
        my_theme_color: None,
        my_badges: Vec::new(),
        my_avatar_ref: None,
        my_banner_ref: None,
        member_profiles: HashMap::new(),
        recent_member_joins: std::collections::VecDeque::new(),
    };

    state
        .mek_cache
        .lock()
        .insert(id.clone(), CryptoMek::from_bytes(mek.key_bytes, mek.generation));
    state.communities.write().insert(id, cs);
}
