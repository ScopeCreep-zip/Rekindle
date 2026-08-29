//! Community overview/detail queries.

use crate::error::{Result, TransportError};
use crate::session::CommunityMembership;

use super::display_map::{channel_to_display, role_to_display};
use super::{CommunityDetail, CommunityOverview, QueryEngine};

impl QueryEngine {
    /// List joined communities with overview metadata.
    ///
    /// Reads each community's governance metadata subkey for name/description
    /// and the member registry index for member count.
    pub async fn list_communities(
        &self,
        memberships: &[CommunityMembership],
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

            let members = self
                .dht
                .registry()
                .read_member_index(&m.registry_key)
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
                member_count: u32::try_from(members.len()).unwrap_or(u32::MAX),
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

        let members = self
            .dht
            .registry()
            .read_member_index(&membership.registry_key)
            .await?;

        Ok(CommunityDetail {
            governance_key: membership.governance_key.clone(),
            name: metadata.name,
            description: metadata.description.unwrap_or_default(),
            owner_pseudonym: metadata.owner_pseudonym,
            created_at: metadata.created_at,
            member_count: u32::try_from(members.len()).unwrap_or(u32::MAX),
            channels: channels.iter().map(channel_to_display).collect(),
            roles: roles.iter().map(role_to_display).collect(),
            our_pseudonym: membership.pseudonym_key.clone(),
            our_roles: membership.role_ids.clone(),
        })
    }
}
