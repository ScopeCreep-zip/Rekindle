//! The governance projection (plan E3.1 B2c): merged governance state laid
//! onto a community's [`CommunityState`] — our role ids, the channels, roles,
//! categories, metadata, MEK generation, the Plate Gate segments, and banned
//! members pruned from the overlay — and its snapshot for SQLite. One body
//! for both hosts: the store runs [`project`] under its write lock
//! (`CommunityStore::project_governance`), and every merge, rebuild and
//! watched change goes through it.

use std::collections::{HashMap, HashSet};

use rekindle_db::repo::communities::snapshot::{
    SnapshotCategory, SnapshotChannel, SnapshotOverwrite, SnapshotRole, StoredGovernance,
};
use rekindle_governance::state::GovernanceState;
use rekindle_types::id::{PseudonymKey, RoleId};

use crate::host::community::state::{
    CategoryInfo, ChannelInfo, ChannelType, CommunityState, RoleDefinition,
};

/// What a projection changed that the shell acts on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GovernanceProjected {
    /// The merge named a record to watch (a channel log, a Plate Gate
    /// segment's records, a channel-segment record) that the previous state
    /// did not.
    pub new_watch_targets: bool,
    /// Our role ids as the merged role assignments name them (empty when
    /// they name none for us), which the session's membership adopts.
    pub my_role_ids: Vec<u32>,
}

/// Every record the community's state names to watch: channel logs,
/// segment governance and registry records, channel-segment records.
fn watch_targets(cs: &CommunityState) -> HashSet<String> {
    cs.channel_log_keys
        .values()
        .cloned()
        .chain(cs.governance_state.iter().flat_map(|g| {
            g.segments
                .iter()
                .flat_map(|s| [s.governance_key.clone(), s.registry_key.clone()])
                .chain(
                    g.channel_segment_records
                        .values()
                        .map(|r| r.record_key.clone()),
                )
        }))
        .collect()
}

fn channel_type(wire: &str) -> ChannelType {
    match wire {
        "voice" => ChannelType::Voice,
        "announcement" => ChannelType::Announcement,
        "forum" => ChannelType::Forum,
        "stage" => ChannelType::Stage,
        "directory" => ChannelType::Directory,
        "media" => ChannelType::Media,
        "events" => ChannelType::Events,
        "dm" => ChannelType::Dm,
        _ => ChannelType::Text,
    }
}

/// The channels merged governance names, ordered by position then name,
/// keeping what is local to each existing channel (unread count, record
/// key, notification settings).
fn project_channels(cs: &mut CommunityState, gov: &GovernanceState) {
    let existing: HashMap<String, ChannelInfo> =
        cs.channels.drain(..).map(|c| (c.id.clone(), c)).collect();
    let mut log_keys = HashMap::new();
    let mut channels: Vec<(u32, ChannelInfo)> = gov
        .channels
        .iter()
        .map(|(id, ch)| {
            let id = hex::encode(id.0);
            let record_key = (!ch.record_key.is_empty()).then(|| ch.record_key.clone());
            if let Some(key) = &record_key {
                log_keys.insert(id.clone(), key.clone());
            }
            let local = existing.get(&id);
            let info = ChannelInfo {
                unread_count: local.map_or(0, |c| c.unread_count),
                message_record_key: local
                    .and_then(|c| c.message_record_key.clone())
                    .or(record_key),
                name: ch.name.clone(),
                channel_type: channel_type(&ch.channel_type),
                category_id: ch.category_id.map(|c| hex::encode(c.0)),
                topic: ch.topic.clone().unwrap_or_default(),
                forum_tags: ch.forum_tags.clone(),
                stage_speakers: Vec::new(),
                stage_moderator: None,
                slowmode_seconds: ch.slowmode_seconds,
                nsfw: ch.nsfw.unwrap_or(false),
                mek_generation: 0,
                notification_level: local
                    .map_or_else(|| "all".to_string(), |c| c.notification_level.clone()),
                notification_sound_ref: local.and_then(|c| c.notification_sound_ref.clone()),
                parent_voice_channel_id: None,
                id,
            };
            (ch.position, info)
        })
        .collect();
    channels.sort_by(|(a_pos, a), (b_pos, b)| a_pos.cmp(b_pos).then_with(|| a.name.cmp(&b.name)));
    cs.channels = channels.into_iter().map(|(_, c)| c).collect();
    cs.channel_log_keys = log_keys;
}

fn project_roles_and_categories(cs: &mut CommunityState, gov: &GovernanceState) {
    cs.roles = gov
        .roles
        .iter()
        .map(|(id, r)| RoleDefinition {
            id: RoleId::to_legacy_u32(*id),
            name: r.name.clone(),
            color: r.color,
            permissions: r.permissions,
            position: r.position.cast_signed(),
            hoist: r.hoist,
            mentionable: r.mentionable,
            self_assignable: r.self_assignable,
            exclusion_group: r.exclusion_group.clone(),
        })
        .collect();
    cs.roles.sort_by_key(|role| role.position);
    cs.categories = gov
        .categories
        .iter()
        .map(|(id, c)| CategoryInfo {
            id: hex::encode(id.0),
            name: c.name.clone(),
            sort_order: c.position.cast_signed(),
        })
        .collect();
    cs.categories.sort_by_key(|category| category.sort_order);
}

/// Our role ids from the merged role assignments.
fn project_my_roles(cs: &mut CommunityState, gov: &GovernanceState) {
    let me = cs
        .my_pseudonym_key
        .as_deref()
        .and_then(|hex| hex::decode(hex).ok())
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .map(PseudonymKey);
    cs.my_role_ids = me
        .and_then(|me| gov.role_assignments.get(&me))
        .map(|role_ids| {
            role_ids
                .iter()
                .copied()
                .map(RoleId::to_legacy_u32)
                .collect()
        })
        .unwrap_or_default();
    cs.my_role_ids.sort_unstable();
}

/// A banned member stops receiving our gossip and leaves the member lists
/// at once (architecture §13).
fn prune_banned(cs: &mut CommunityState, gov: &GovernanceState) {
    if gov.bans.is_empty() {
        return;
    }
    let banned: HashSet<String> = gov.bans.iter().map(|p| hex::encode(p.0)).collect();
    if let Some(gossip) = cs.gossip.as_mut() {
        gossip.peers.retain(|key, _| !banned.contains(key));
        gossip.online_members.retain(|key, _| !banned.contains(key));
    }
    cs.known_members.retain(|key| !banned.contains(key));
}

/// Lay `gov` onto `cs` and install it as the community's governance state.
pub fn project(cs: &mut CommunityState, gov: GovernanceState) -> GovernanceProjected {
    let prior = watch_targets(cs);
    project_my_roles(cs, &gov);
    project_channels(cs, &gov);
    project_roles_and_categories(cs, &gov);
    if let Some(meta) = &gov.metadata {
        cs.name.clone_from(&meta.name);
        cs.description.clone_from(&meta.description);
        cs.icon_hash.clone_from(&meta.icon_hash);
        cs.banner_hash.clone_from(&meta.banner_hash);
    }
    cs.mek_generation = gov.mek_generation;
    prune_banned(cs, &gov);
    super::event_rsvps::project(cs, &gov);
    // Segment 0 stays implicit (`rekindle_governance_runtime::segments`).
    cs.segments = gov
        .segments
        .iter()
        .filter(|s| s.segment_index != 0)
        .map(|s| rekindle_types::presence::SegmentDescriptor {
            segment_index: s.segment_index,
            registry_key: s.registry_key.clone(),
            governance_key: s.governance_key.clone(),
            slot_range_start: s.slot_range_start,
            slot_range_end: s.slot_range_end,
        })
        .collect();
    cs.governance_state = Some(gov);
    let new_watch_targets = !watch_targets(cs).is_subset(&prior);
    GovernanceProjected {
        new_watch_targets,
        my_role_ids: cs.my_role_ids.clone(),
    }
}

/// The SQLite snapshot of a projected community at `lamport_clock`; `None`
/// before its governance has merged.
#[must_use]
pub fn snapshot(cs: &CommunityState, lamport_clock: i64) -> Option<StoredGovernance> {
    let gov = cs.governance_state.as_ref()?;
    let mek_generation = i64::try_from(cs.mek_generation).unwrap_or(i64::MAX);
    let mut channels: Vec<SnapshotChannel> = gov
        .channels
        .iter()
        .map(|(id, ch)| {
            let id = hex::encode(id.0);
            SnapshotChannel {
                my_sequence: cs
                    .channel_sequences
                    .get(&id)
                    .map_or(0, |s| i64::try_from(*s).unwrap_or(i64::MAX)),
                id,
                name: ch.name.clone(),
                channel_type: ch.channel_type.clone(),
                sort_order: i64::from(ch.position),
                category_id: ch.category_id.map(|c| hex::encode(c.0)),
                topic: ch.topic.clone().unwrap_or_default(),
                slowmode_seconds: i64::from(ch.slowmode_seconds.unwrap_or(0)),
                nsfw: ch.nsfw.unwrap_or(false),
                record_key: (!ch.record_key.is_empty()).then(|| ch.record_key.clone()),
                mek_generation,
            }
        })
        .collect();
    channels.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut roles: Vec<SnapshotRole> = gov
        .roles
        .iter()
        .map(|(id, r)| SnapshotRole {
            role_id: RoleId::to_legacy_u32(*id),
            name: r.name.clone(),
            color: r.color,
            permissions: r.permissions,
            position: r.position,
            hoist: r.hoist,
            mentionable: r.mentionable,
            self_assignable: r.self_assignable,
            exclusion_group: r.exclusion_group.clone(),
        })
        .collect();
    roles.sort_by(|a, b| {
        a.position
            .cmp(&b.position)
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut categories: Vec<SnapshotCategory> = gov
        .categories
        .iter()
        .map(|(id, c)| SnapshotCategory {
            id: hex::encode(id.0),
            name: c.name.clone(),
            sort_order: i64::from(c.position),
        })
        .collect();
    categories.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut overwrites: Vec<SnapshotOverwrite> = gov
        .overwrites
        .iter()
        .map(|((channel, target), o)| SnapshotOverwrite {
            channel_id: hex::encode(channel.0),
            target_type: o.target_type.clone(),
            target_id: target.clone(),
            allow: o.allow,
            deny: o.deny,
        })
        .collect();
    overwrites.sort_by(|a, b| {
        (&a.channel_id, &a.target_type, &a.target_id).cmp(&(
            &b.channel_id,
            &b.target_type,
            &b.target_id,
        ))
    });
    let mut my_role_ids = cs.my_role_ids.clone();
    my_role_ids.sort_unstable();
    let metadata = gov.metadata.as_ref();
    Some(StoredGovernance {
        name: cs.name.clone(),
        description: cs.description.clone(),
        icon_hash: metadata.and_then(|m| m.icon_hash.clone()),
        banner_hash: metadata.and_then(|m| m.banner_hash.clone()),
        my_role_ids,
        mek_generation,
        lamport_clock,
        channels,
        roles,
        categories,
        overwrites,
    })
}
