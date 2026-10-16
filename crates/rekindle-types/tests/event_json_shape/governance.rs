//! `GovernanceEvent` samples.

use rekindle_types::display::{CategoryDisplay, ChannelOverviewDisplay, RoleDisplay};
use rekindle_types::presence::SegmentDescriptor;
use rekindle_types::subscription_events::{GovernanceEvent, SubscriptionEvent};

use crate::{channel, community, id, pk, rk, text, Sample};

pub const NAMES: &[&str] = &[
    "metadataChanged",
    "channelsChanged",
    "rolesChanged",
    "segmentsChanged",
    "bansChanged",
    "inviteCreated",
    "inviteUsed",
    "inviteRevoked",
    "channelPermissionsChanged",
    "governanceSubkeyUpdated",
    "governanceRebuilt",
];

pub fn variant_name(e: &GovernanceEvent) -> &'static str {
    match e {
        GovernanceEvent::MetadataChanged { .. } => "metadataChanged",
        GovernanceEvent::ChannelsChanged { .. } => "channelsChanged",
        GovernanceEvent::RolesChanged { .. } => "rolesChanged",
        GovernanceEvent::SegmentsChanged { .. } => "segmentsChanged",
        GovernanceEvent::BansChanged { .. } => "bansChanged",
        GovernanceEvent::InviteCreated { .. } => "inviteCreated",
        GovernanceEvent::InviteUsed { .. } => "inviteUsed",
        GovernanceEvent::InviteRevoked { .. } => "inviteRevoked",
        GovernanceEvent::ChannelPermissionsChanged { .. } => "channelPermissionsChanged",
        GovernanceEvent::GovernanceSubkeyUpdated { .. } => "governanceSubkeyUpdated",
        GovernanceEvent::GovernanceRebuilt { .. } => "governanceRebuilt",
    }
}

fn ev(name: &str, e: GovernanceEvent) -> Sample {
    (
        format!("governance.{name}"),
        SubscriptionEvent::Governance(e),
    )
}

fn invite_hash() -> String {
    "7".repeat(64)
}

pub fn samples() -> Vec<Sample> {
    let mut all = structure();
    all.extend(invites_and_subkeys());
    all
}

fn structure() -> Vec<Sample> {
    vec![
        ev(
            "metadataChanged",
            GovernanceEvent::MetadataChanged {
                community: community(),
                name: Some(text("LAN Party")),
                description: Some(text("Friday night frags")),
                icon_hash: Some("1".repeat(64)),
                banner_hash: Some("2".repeat(64)),
            },
        ),
        ev(
            "channelsChanged",
            GovernanceEvent::ChannelsChanged {
                community: community(),
                channels: vec![ChannelOverviewDisplay {
                    id: channel(),
                    name: text("general"),
                    kind: text("text"),
                    category_id: Some(id("cat_", '4')),
                    topic: text("Say hi"),
                    mek_generation: 5,
                    log_key: Some(rk('h')),
                    sort_order: 1,
                    slowmode_seconds: Some(30),
                }],
                categories: vec![CategoryDisplay {
                    id: id("cat_", '4'),
                    name: text("Text Channels"),
                    sort_order: 0,
                }],
            },
        ),
        ev(
            "rolesChanged",
            GovernanceEvent::RolesChanged {
                community: community(),
                roles: vec![RoleDisplay {
                    id: 3,
                    name: text("Moderator"),
                    color: 0x00_FF_00,
                    permissions: rekindle_types::permissions::ALL,
                    position: 1,
                    hoist: true,
                    mentionable: true,
                    self_assignable: false,
                    exclusion_group: Some(text("rank")),
                }],
            },
        ),
        ev(
            "segmentsChanged",
            GovernanceEvent::SegmentsChanged {
                community: community(),
                segments: vec![SegmentDescriptor {
                    segment_index: 1,
                    registry_key: rk('r'),
                    governance_key: rk('s'),
                    slot_range_start: 255,
                    slot_range_end: 510,
                }],
            },
        ),
        ev(
            "bansChanged",
            GovernanceEvent::BansChanged {
                community: community(),
            },
        ),
    ]
}

fn invites_and_subkeys() -> Vec<Sample> {
    vec![
        ev(
            "inviteCreated",
            GovernanceEvent::InviteCreated {
                community: community(),
                code_hash: invite_hash(),
                created_by: pk('a'),
                max_uses: Some(10),
                uses: 0,
                expires_at: Some(1_760_086_400_000),
                created_at: 1_760_000_000_000,
            },
        ),
        ev(
            "inviteUsed",
            GovernanceEvent::InviteUsed {
                community: community(),
                code_hash: invite_hash(),
                uses: 1,
            },
        ),
        ev(
            "inviteRevoked",
            GovernanceEvent::InviteRevoked {
                community: community(),
                code_hash: invite_hash(),
            },
        ),
        ev(
            "channelPermissionsChanged",
            GovernanceEvent::ChannelPermissionsChanged {
                community: community(),
                channel: channel(),
            },
        ),
        ev(
            "governanceSubkeyUpdated",
            GovernanceEvent::GovernanceSubkeyUpdated {
                community: community(),
                subkey_index: 4,
                lamport_ts: 77,
            },
        ),
        ev(
            "governanceRebuilt",
            GovernanceEvent::GovernanceRebuilt {
                community: community(),
            },
        ),
    ]
}
