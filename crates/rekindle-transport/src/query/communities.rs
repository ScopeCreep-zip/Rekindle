//! Community overview/detail queries.
//!
//! Member counts arrive as a parameter rather than being read here.
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

use super::display_map::{channel_to_display, role_to_display};
use super::{CommunityDetail, CommunityOverview, QueryEngine};

impl QueryEngine {
    /// List joined communities with overview metadata.
    ///
    /// Reads each community's governance metadata subkey for
    /// name/description. `member_counts` is keyed by governance key; a
    /// community missing from it reports zero.
    pub async fn list_communities(
        &self,
        memberships: &[CommunityMembership],
        member_counts: &HashMap<String, u32>,
    ) -> Result<Vec<CommunityOverview>> {
        let mut result = Vec::with_capacity(memberships.len());

        for m in memberships {
            let metadata = self
                .dht
                .governance()
                .read_metadata(&m.governance_key)
                .await?;

            let channels = self
                .dht
                .governance()
                .read_channels(&m.governance_key)
                .await
                .unwrap_or_default();

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
                channel_count: u32::try_from(channels.len()).unwrap_or(u32::MAX),
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

        let channels = self
            .dht
            .governance()
            .read_channels(&membership.governance_key)
            .await?;

        let roles = self
            .dht
            .governance()
            .read_roles(&membership.governance_key)
            .await?;

        Ok(CommunityDetail {
            governance_key: membership.governance_key.clone(),
            name: metadata.name,
            description: metadata.description.unwrap_or_default(),
            owner_pseudonym: metadata.owner_pseudonym,
            created_at: metadata.created_at,
            member_count,
            channels: channels.iter().map(channel_to_display).collect(),
            roles: roles.iter().map(role_to_display).collect(),
            our_pseudonym: membership.pseudonym_key.clone(),
            our_roles: membership.role_ids.clone(),
        })
    }
}
