//! Community overview/detail queries.
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

use crate::error::{Result, TransportError};
use crate::session::CommunityMembership;

use super::{ChannelOverviewDisplay, CommunityDetail, CommunityOverview, QueryEngine, RoleDisplay};

impl QueryEngine {
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
    pub async fn list_communities(
        &self,
        memberships: &[CommunityMembership],
        member_counts: &HashMap<String, u32>,
        channel_counts: &HashMap<String, u32>,
    ) -> Result<Vec<CommunityOverview>> {
        let mut result = Vec::with_capacity(memberships.len());

        for m in memberships {
            let metadata = self
                .dht
                .governance()
                .read_metadata(&m.governance_key)
                .await?;

            let (name, description) = match metadata {
                Some(meta) => (meta.name, meta.description.unwrap_or_default()),
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

        Ok(result)
    }

    /// Detailed info about a single community.
    pub async fn community_detail(
        &self,
        membership: &CommunityMembership,
        member_count: u32,
        channels: Vec<ChannelOverviewDisplay>,
        roles: Vec<RoleDisplay>,
    ) -> Result<CommunityDetail> {
        // Ensure governance and registry records are open for reading.
        // Records may have been closed since community creation/join.
        let _ = crate::broadcast::dht::record::open_readonly(
            self.dht.routing_context(),
            &membership.governance_key,
        )
        .await;
        let _ = crate::broadcast::dht::record::open_readonly(
            self.dht.routing_context(),
            &membership.registry_key,
        )
        .await;

        let metadata = self
            .dht
            .governance()
            .read_metadata(&membership.governance_key)
            .await?
            .ok_or_else(|| TransportError::DhtError {
                reason: format!(
                    "governance metadata not found for {}",
                    membership.governance_key
                ),
            })?;

        Ok(CommunityDetail {
            governance_key: membership.governance_key.clone(),
            name: metadata.name,
            description: metadata.description.unwrap_or_default(),
            owner_pseudonym: metadata.owner_pseudonym,
            created_at: metadata.created_at,
            member_count,
            channels,
            roles,
            our_pseudonym: membership.pseudonym_key.clone(),
            our_roles: membership.role_ids.clone(),
        })
    }
}
