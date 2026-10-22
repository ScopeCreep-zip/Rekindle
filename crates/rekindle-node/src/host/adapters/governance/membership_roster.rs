//! The community roster on both hosts (plan E3.1 D1b): a member's own
//! `MemberJoined` adds its `community_members` row, its `MemberLeave` (or a
//! moderator's kick or ban, through [`HostGovernance::forget_member`])
//! removes it and records the departure for the community's analytics, and
//! a join announces us to the mesh.

use std::time::Duration;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance_runtime::GovernanceRuntimeDeps;

use super::community_sync::status_str;
use super::HostGovernance;
use crate::host::community::OnlineMember;

/// How long a join waits for its first gossip peers before announcing.
const JOIN_SETTLE: Duration = Duration::from_secs(5);

impl HostGovernance {
    /// A member announced its own join: known, a gossip peer when online
    /// with a route, and on the stored roster.
    pub(super) fn on_member_joined(
        &self,
        community_id: &str,
        pseudonym_hex: &str,
        display_name: &str,
        role_ids: &[u32],
        status: &str,
        route_blob: Option<&[u8]>,
    ) {
        self.ctx
            .communities
            .extend_known_members(community_id, vec![pseudonym_hex.to_string()]);
        if status != "offline" {
            if let Some(route) = route_blob.filter(|r| !r.is_empty()) {
                self.ctx.communities.note_online_member(
                    community_id,
                    pseudonym_hex,
                    OnlineMember {
                        route_blob: route.to_vec(),
                        status: status.to_string(),
                        last_seen: self.ctx.clock.now_secs(),
                        ..OnlineMember::default()
                    },
                );
            }
        }
        let Some((owner, db)) = self.owner_and_db() else {
            return;
        };
        let (community, pseudonym, name, roles) = (
            community_id.to_string(),
            pseudonym_hex.to_string(),
            display_name.to_string(),
            role_ids.to_vec(),
        );
        let joined_at = self.ctx.clock.now_secs().cast_signed();
        db.fire("member joined", move |conn| {
            rekindle_db::repo::members::insert_if_absent(
                conn, &owner, &community, &pseudonym, &name, &roles, joined_at,
            )
        });
    }

    /// A member departed (left, or was kicked or banned): forgotten in the
    /// store, its roster row deleted and the departure recorded.
    pub fn forget_member(&self, community_id: &str, pseudonym_hex: &str) {
        self.ctx
            .communities
            .forget_member(community_id, pseudonym_hex);
        let Some((owner, db)) = self.owner_and_db() else {
            return;
        };
        let (community, pseudonym) = (community_id.to_string(), pseudonym_hex.to_string());
        let left_at = self.ctx.clock.now_ms_i64();
        db.fire("member departed", move |conn| {
            rekindle_db::repo::members::delete(conn, &owner, &community, &pseudonym)?;
            rekindle_db::repo::members::record_leave(conn, &owner, &community, &pseudonym, left_at)
        });
    }

    /// Announce our join to the community's mesh once its first peers have
    /// had [`JOIN_SETTLE`] to appear.
    pub(crate) fn announce_join(&self, community_id: &str) {
        let host = self.clone();
        let community = community_id.to_string();
        let stop = self.scope().token();
        self.scope().spawn_or_drop("join announcement", async move {
            if stop
                .run_until_cancelled(tokio::time::sleep(JOIN_SETTLE))
                .await
                .is_none()
            {
                return;
            }
            let (_, mine) = host.ctx.communities.registry_and_pseudonym(&community);
            let Some(pseudonym_key) = mine else {
                return;
            };
            let route = host.our_route_blob_impl();
            let joined = CommunityEnvelope::Control(ControlPayload::MemberJoined {
                pseudonym_key,
                display_name: host.identity_display_name_impl(),
                role_ids: vec![0],
                status: status_str(host.ctx.own_presence.status()).to_string(),
                route_blob: (!route.is_empty()).then_some(route),
            });
            host.send_to_mesh_impl(&community, &joined);
            tracing::info!(community = %community, "join announced to the mesh");
        });
    }
}
