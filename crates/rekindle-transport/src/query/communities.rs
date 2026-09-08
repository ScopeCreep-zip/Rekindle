//! Community overview/detail — pure mapping, no queries left.
//!
//! Every fact these produce now arrives as a parameter, because every
//! one of them lives in merged governance and this layer cannot reach
//! it. What they used to read was the v1.0 governance manifest — member
//! index for the count, channels and roles subkeys, metadata subkey —
//! none of which has had a writer since those became CRDT entries. A
//! renamed community kept its old name here forever, and a
//! CRDT-created channel never appeared at all.
//!
//! Taking them as arguments rather than defaulting is deliberate: a
//! forgotten one is a compile error, not a UI that quietly reports an
//! empty community.
//!
//! Member counts, channels and roles all arrive as parameters rather
//! than being read here. Each is an answer the merged CRDT holds and
//! this layer cannot reach.
//!
//! These used to call `read_member_index` — one aggregate subkey
//! asserting facts about every member, which `o_cnt: 0` gives nobody
//! the authority to write. The count now comes from the caller's
//! roster, which the presence poll materialised from individually
//! W26-verified rows. Taking it as an argument rather than defaulting
//! to zero is deliberate: a forgotten count is then a compile error,
//! not a UI that quietly reports an empty community.

use std::collections::HashMap;

use crate::session::CommunityMembership;

use super::{ChannelOverviewDisplay, CommunityDetail, CommunityOverview, RoleDisplay};

/// The community facts merged governance holds, supplied by the caller.
///
/// Replaces reading the v1.0 governance-manifest metadata subkey, which
/// nothing has written since `CommunityMeta` became a governance entry
/// — a renamed community kept its old name here forever.
#[derive(Debug, Clone, Default)]
pub struct CommunityMetaSummary {
    pub name: String,
    pub description: String,
    /// `GovernanceState.creator` — the genesis author. There is no
    /// transferable "owner" under flat governance; see
    /// `rekindle_governance_runtime::ownership`.
    pub creator_pseudonym: String,
    /// Wall-clock creation time, if the caller has one.
    ///
    /// The CRDT does not carry it: entries are ordered by Lamport clock,
    /// and a self-reported timestamp from a peer is unverifiable. Hosts
    /// that recorded it locally at join or create can supply it; `0`
    /// means "not known", not "the epoch".
    pub created_at: u64,
}

/// List joined communities with overview metadata.
///
/// Reads each community's governance metadata subkey for
/// name/description. `member_counts` and `channel_counts` are keyed
/// by governance key; a community missing from either reports zero.
///
/// Both come from the caller for the same reason: they are answers
/// the merged CRDT holds, and this layer has no access to it. The
/// channel count used to come from the v1.0 manifest channels
/// subkey, which nothing has written since channels became
/// `ChannelCreated` entries.
#[must_use]
pub fn list_communities<S: std::hash::BuildHasher>(
    memberships: &[CommunityMembership],
    member_counts: &HashMap<String, u32, S>,
    channel_counts: &HashMap<String, u32, S>,
    metadata: &HashMap<String, CommunityMetaSummary, S>,
) -> Vec<CommunityOverview> {
    let mut result = Vec::with_capacity(memberships.len());

    for m in memberships {
        let (name, description) = match metadata.get(&m.governance_key) {
            Some(meta) => (meta.name.clone(), meta.description.clone()),
            // The persisted membership name is the fallback for a
            // community whose `CommunityMeta` has not merged yet.
            None => (m.community_name.clone(), String::new()),
        };

        result.push(CommunityOverview {
            governance_key: m.governance_key.clone(),
            name,
            description,
            member_count: member_counts
                .get(&m.governance_key)
                .copied()
                .unwrap_or_default(),
            channel_count: channel_counts
                .get(&m.governance_key)
                .copied()
                .unwrap_or_default(),
            our_pseudonym: m.pseudonym_key.clone(),
        });
    }

    result
}

/// Detailed info about a single community.
#[must_use]
pub fn community_detail(
    membership: &CommunityMembership,
    member_count: u32,
    channels: Vec<ChannelOverviewDisplay>,
    roles: Vec<RoleDisplay>,
    meta: CommunityMetaSummary,
) -> CommunityDetail {
    CommunityDetail {
        governance_key: membership.governance_key.clone(),
        name: meta.name,
        description: meta.description,
        owner_pseudonym: meta.creator_pseudonym,
        created_at: meta.created_at,
        member_count,
        channels,
        roles,
        our_pseudonym: membership.pseudonym_key.clone(),
        our_roles: membership.role_ids.clone(),
    }
}
