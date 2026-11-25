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

    /// Where one of our governance ops stands (`op` is its hash, hex), from
    /// the backend: `Pending` while it is applied, gossiped and queued on
    /// the governance-op stream (announced again at every unlock while its
    /// row is queued), `Landed` once it is on our member record and the
    /// converged op set applies it, `Failed` when the community refused or
    /// quarantined it at convergence, or its row expired, wedged or was
    /// discarded. The UI shows a pending op as pending, with no retry
    /// (plan E3.2 S6b, review RV7 M2).
    OpPublication {
        community: String,
        op: String,
        state: OpPublicationState,
    },

    /// The unlock barrier (plan E3.2 S7d-keys.8): whether it is catching
    /// up and every governance write it holds, in full on every change (no
    /// deltas); cleared by one `catching_up: false, pending: []`, after
    /// which each op's own events follow as usual.
    CatchUp {
        community: String,
        catching_up: bool,
        pending: Vec<crate::display_catch_up::PendingGovOp>,
    },

    /// The role table changed — the full new table, not a delta.
    RolesChanged {
        community: String,
        roles: Vec<RoleDisplay>,
    },

    /// Plate Gate (architecture §15): the expansion-segment list changed —
    /// the full new list (segment 0 is implicit, never included), not a
    /// delta. Reuses `SegmentDescriptor` rather than an event-only DTO,
    /// same reasoning as `RolesChanged`/`RoleDisplay`.
    SegmentsChanged {
        community: String,
        segments: Vec<crate::presence::SegmentDescriptor>,
    },

    /// The ban list changed.
    BansChanged { community: String },

    /// An invite was created.
    InviteCreated {
        community: String,
        /// The governance invite id, 32 lowercase hex: what a revoke names.
        invite_id: String,
        code_hash: String,
        created_by: String,
        /// Redemptions allowed, `1..=100` (plan E3.2 (d)).
        max_uses: u32,
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

    /// An invite we created reached its expiry (the Host's timer).
    InviteExpired {
        community: String,
        code_hash: String,
    },

    /// A channel's permission overwrites changed.
    /// Triggered by: the Host, when the merged governance holds a
    /// different overwrite set for the channel (`permission_events.rs`).
    ChannelPermissionsChanged { community: String, channel: String },

    /// What `member` may do changed: a role, a role assignment, an
    /// overwrite or a timeout moved its base or any channel's effective
    /// permissions (plan E3.2 S9). A frontend re-reads its permission
    /// flags when `member` is us, and re-renders the member otherwise.
    /// Triggered by: the Host, from the merged governance
    /// (`permission_events.rs`).
    EffectivePermissionsChanged { community: String, member: String },

    /// The CRDT governance state was rebuilt from the DHT.
    ///
    /// No payload on purpose: a rebuild moves channels, roles, members
    /// and permissions together, so re-reading is the correct response
    /// rather than shipping four lists.
    GovernanceRebuilt { community: String },

    /// A governance op's fate moved to quarantined (plan E3.2 S7c.5): it is
    /// held and shown, never applied. Published once per op, when the
    /// merge first sets it aside.
    GovernanceOpQuarantined {
        community: String,
        /// The op's hash, hex.
        op: String,
        /// Its author's community pseudonym, hex.
        author: String,
        reason: OpQuarantineReason,
        /// The hash (hex) of the op that caused it: the removal, or the
        /// other `Checkpoint` op; `None` for an op this reader cannot
        /// interpret, which quarantines the community (D15), and for a
        /// claim of a record another member holds.
        cause: Option<String>,
    },

    /// A community joined from a checkpoint hint finished its backfill and
    /// the full op set's state at the hint's cut is not the hinted
    /// snapshot: a strict majority endorsed a false checkpoint (plan E3.2
    /// S7c.8 (1), threat T8). The community stays on the backfilled state.
    CheckpointSnapshotMismatch {
        community: String,
        /// The hint's checkpoint height.
        height: u64,
        /// The hint's checkpoint digest, hex.
        digest: String,
    },

    /// A successor community both sides link (ADR 0011 "Successor
    /// community", plan E3.2 S7s.3e): an applied `CommunitySuccessor` op in
    /// this community names `new_anchor`, and that anchor's verified
    /// `GenesisCert` names this community as its `predecessor` (for a
    /// transfer, its creator set is the acceptance's links, each signed by
    /// the creator's old pseudonym). Announced once per op; two valid
    /// successors are both offered. `signer` is the op's author, hex;
    /// `creators` the new community's creator set, hex.
    CommunitySuccessorOffered {
        community: String,
        new_anchor: String,
        signer: String,
        reason: crate::governance::SuccessorReason,
        creators: Vec<String>,
    },

    /// A transfer's link request asks us (lead decision (46), plan E3.2
    /// S7s.4c): `recipient`, the offer's recipient, asks this member, a
    /// creator of the offer's set, to link into its successor
    /// `new_anchor`. Announced once per request while it is unanswered and
    /// its offer stands; answered with intent `AnswerSuccessorLink`.
    /// `request` is the request op's hash and `recipient` a pseudonym, hex.
    SuccessorLinkRequested {
        community: String,
        request: String,
        recipient: String,
        new_anchor: String,
    },
}

/// Why a governance op is quarantined
/// ([`GovernanceEvent::GovernanceOpQuarantined`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpQuarantineReason {
    /// "after its author's removal".
    AfterAuthorRemoval,
    /// "two checkpoint endorsements in one round".
    TwoCheckpointEndorsementsInOneRound,
    /// "checkpoint equivocation".
    CheckpointEquivocation,
    /// "member record already claimed": a join or record move naming a
    /// member record another member holds (ADR 0011, one member per
    /// record); `cause` is `None`. The claimant's own refused join tells
    /// it to enrol again with a record of its own.
    RecordClaimed,
    /// "quarantined because <cause> was": its authority (a role grant, an
    /// invite, a channel or a ban it acts on) came from `cause`, an op
    /// quarantined after its author's fork, and fails without it (ADR 0022
    /// rule 5, plan E3.2 S7d-keys.2a).
    AuthorityQuarantined,
    /// "its device key was retired or forked before it" (ADR 0022 rules
    /// 4, 5): its key is not effective for its author at its deps.
    DeviceKeyRetired,
    /// "its device key was endorsed past six devices" (rule 6).
    DeviceKeyInert,
    /// "its device key is not among its author's first six" (rule 6).
    DeviceKeyOverCap,
    /// "its device key answers a fork with no re-admission" (rules 7, 10).
    DeviceKeyNeedsReadmit,
    /// "its device key names another device's record" (rule 3).
    DeviceKeyRecordTaken,
    /// A validly signed op of a kind this reader's schema does not have
    /// (D15): the community is quarantined, likely by a newer version's
    /// op; `cause` is `None`.
    UnknownOpKind,
    /// A validly signed op carrying fields this reader's schema does not
    /// have (D15): the community is quarantined as for
    /// [`Self::UnknownOpKind`].
    UnknownOpVersion,
}

/// An own governance op's publication state ([`GovernanceEvent::OpPublication`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "reason")]
pub enum OpPublicationState {
    /// Applied, gossiped and queued; the outbox lands it, never the user.
    Pending,
    /// On our member record, and the converged op set applies it.
    Landed,
    /// It will not stand, for the reason given.
    Failed(String),
}

impl GovernanceEvent {
    /// The community this event belongs to. Every variant has one.
    #[must_use]
    pub fn community(&self) -> &str {
        match self {
            Self::MetadataChanged { community, .. }
            | Self::ChannelsChanged { community, .. }
            | Self::OpPublication { community, .. }
            | Self::CatchUp { community, .. }
            | Self::RolesChanged { community, .. }
            | Self::SegmentsChanged { community, .. }
            | Self::BansChanged { community }
            | Self::InviteCreated { community, .. }
            | Self::InviteUsed { community, .. }
            | Self::InviteRevoked { community, .. }
            | Self::InviteExpired { community, .. }
            | Self::ChannelPermissionsChanged { community, .. }
            | Self::EffectivePermissionsChanged { community, .. }
            | Self::GovernanceRebuilt { community }
            | Self::GovernanceOpQuarantined { community, .. }
            | Self::CheckpointSnapshotMismatch { community, .. }
            | Self::CommunitySuccessorOffered { community, .. }
            | Self::SuccessorLinkRequested { community, .. } => community,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role() -> RoleDisplay {
        RoleDisplay {
            id: "03".repeat(16),
            name: "Mod".into(),
            color: 0x00_FF_00,
            tag_colours: crate::dto::community::RoleTagColours {
                dark: 0x00_FF_00,
                light: 0x00_6B_00,
            },
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
        assert_eq!(
            GovernanceEvent::SegmentsChanged {
                community: "c".into(),
                segments: Vec::new(),
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
                invite_id: "0a".repeat(16),
                code_hash: "hash".into(),
                created_by: "pk".into(),
                max_uses: 5,
                uses: 0,
                expires_at: None,
                created_at: 1,
            },
            GovernanceEvent::SegmentsChanged {
                community: "c".into(),
                segments: vec![crate::presence::SegmentDescriptor {
                    segment_index: 1,
                    registry_key: "reg1".into(),
                    anchor: "gov1".into(),
                    slot_range_start: 255,
                    slot_range_end: 510,
                }],
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
