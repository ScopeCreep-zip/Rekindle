//! Community governance events — metadata, channels, roles, invites, permissions.
//!
//! Triggered by gossip `ControlPayload` variants and DHT `ValueChange`
//! on the governance manifest record.
//!
//! ## Why these carry payloads
//!
//! Every variant here used to be `{ community }` alone — "something
//! changed, go re-read it". The desktop's parallel `CommunityEvent`
//! shipped the new state inline, and converging onto the bare form
//! would have cost an IPC round trip per change plus a window where the
//! store is visibly stale.
//!
//! So the payloads moved into Tier 1 instead. They reuse the existing
//! [`RoleDisplay`] / [`ChannelOverviewDisplay`] types rather than
//! introducing event-only DTOs, which is what makes a role rendered
//! from a `RolesChanged` event and one rendered from `CommunityDetail`
//! the same shape.
//!
//! [`GovernanceRebuilt`](GovernanceEvent::GovernanceRebuilt) is the one
//! that stays a bare signal, and deliberately: a CRDT rebuild changes
//! channels, roles, members and permissions at once, so there is no
//! single payload to carry and re-reading is the correct response.
//!
//! ## Wire constraints
//!
//! Same as [`super::presence`]: postcard on the daemon IPC, so no
//! `#[serde(flatten)]` and no `tag = "..."` enums.

use serde::{Deserialize, Serialize};

use crate::display::{CategoryDisplay, ChannelOverviewDisplay, RoleDisplay};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum GovernanceEvent {
    /// Community name, description, or imagery changed.
    ///
    /// Each field is `None` when that attribute was not part of the
    /// change, so a consumer applies only what it is given rather than
    /// blanking the rest.
    MetadataChanged {
        community: String,
        name: Option<String>,
        description: Option<String>,
        icon_hash: Option<String>,
        banner_hash: Option<String>,
    },

    /// The channel tree changed — the full new tree, not a delta.
    ChannelsChanged {
        community: String,
        channels: Vec<ChannelOverviewDisplay>,
        categories: Vec<CategoryDisplay>,
    },

    /// The role table changed — the full new table, not a delta.
    RolesChanged {
        community: String,
        roles: Vec<RoleDisplay>,
    },

    /// The ban list changed.
    BansChanged { community: String },

    /// An invite was created.
    InviteCreated {
        community: String,
        code_hash: String,
        created_by: String,
        max_uses: Option<u32>,
        uses: u32,
        expires_at: Option<u64>,
        created_at: u64,
    },

    /// An invite was redeemed; `uses` is its new count.
    InviteUsed {
        community: String,
        code_hash: String,
        uses: u32,
    },

    /// An invite was revoked.
    InviteRevoked {
        community: String,
        code_hash: String,
    },

    /// A channel's permission overwrites changed.
    ChannelPermissionsChanged { community: String, channel: String },

    /// One governance subkey was written.
    GovernanceSubkeyUpdated {
        community: String,
        subkey_index: u32,
        lamport_ts: u64,
    },

    /// The CRDT governance state was rebuilt from the DHT.
    ///
    /// No payload on purpose: a rebuild moves channels, roles, members
    /// and permissions together, so re-reading is the correct response
    /// rather than shipping four lists.
    GovernanceRebuilt { community: String },
}

impl GovernanceEvent {
    /// The community this event belongs to. Every variant has one.
    #[must_use]
    pub fn community(&self) -> &str {
        match self {
            Self::MetadataChanged { community, .. }
            | Self::ChannelsChanged { community, .. }
            | Self::RolesChanged { community, .. }
            | Self::BansChanged { community }
            | Self::InviteCreated { community, .. }
            | Self::InviteUsed { community, .. }
            | Self::InviteRevoked { community, .. }
            | Self::ChannelPermissionsChanged { community, .. }
            | Self::GovernanceSubkeyUpdated { community, .. }
            | Self::GovernanceRebuilt { community } => community,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role() -> RoleDisplay {
        RoleDisplay {
            id: 3,
            name: "Mod".into(),
            color: 0x00_FF_00,
            permissions: crate::permissions::ALL,
            position: 1,
            hoist: true,
            mentionable: false,
            self_assignable: false,
            exclusion_group: Some("rank".into()),
        }
    }

    #[test]
    fn every_variant_names_its_community() {
        assert_eq!(
            GovernanceEvent::RolesChanged {
                community: "c".into(),
                roles: vec![role()],
            }
            .community(),
            "c"
        );
        assert_eq!(
            GovernanceEvent::GovernanceRebuilt {
                community: "c".into()
            }
            .community(),
            "c"
        );
    }

    /// The daemon IPC is postcard: `flatten` and `tag = "..."` fail
    /// there at runtime, so only a real round trip catches them. The
    /// payloads make this more than a formality — `RoleDisplay` and
    /// `ChannelOverviewDisplay` now ride inside these events.
    #[test]
    fn postcard_round_trips_payload_variants() {
        let events = vec![
            GovernanceEvent::MetadataChanged {
                community: "c".into(),
                name: Some("New".into()),
                description: None,
                icon_hash: None,
                banner_hash: Some("bh".into()),
            },
            GovernanceEvent::RolesChanged {
                community: "c".into(),
                roles: vec![role()],
            },
            GovernanceEvent::ChannelsChanged {
                community: "c".into(),
                channels: vec![ChannelOverviewDisplay {
                    id: "ch".into(),
                    name: "general".into(),
                    kind: "text".into(),
                    category_id: None,
                    topic: String::new(),
                    mek_generation: 4,
                    log_key: None,
                    sort_order: 0,
                    slowmode_seconds: Some(30),
                }],
                categories: vec![CategoryDisplay {
                    id: "cat".into(),
                    name: "Text".into(),
                    sort_order: 0,
                }],
            },
            GovernanceEvent::InviteCreated {
                community: "c".into(),
                code_hash: "hash".into(),
                created_by: "pk".into(),
                max_uses: Some(5),
                uses: 0,
                expires_at: None,
                created_at: 1,
            },
        ];
        for event in events {
            let bytes = postcard::to_allocvec(&event).expect("postcard encode");
            let back: GovernanceEvent = postcard::from_bytes(&bytes).expect("postcard decode");
            assert_eq!(format!("{event:?}"), format!("{back:?}"));
        }
    }

    /// `permissions::ALL` must stay inside JavaScript's safe-integer
    /// range while `RoleDisplay.permissions` is a plain `u64` — see the
    /// note on that field. Adding permission bit 53 breaks this.
    #[test]
    fn permissions_all_survives_a_json_round_trip() {
        let json = serde_json::to_string(&role()).unwrap();
        let back: RoleDisplay = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.permissions,
            crate::permissions::ALL,
            "ALL exceeded 2^53 — RoleDisplay.permissions needs a string encoding"
        );
        let js_max_safe_integer = (1u64 << 53) - 1;
        assert!(
            back.permissions <= js_max_safe_integer,
            "permission bits grew past JavaScript's safe integer range"
        );
    }
}
