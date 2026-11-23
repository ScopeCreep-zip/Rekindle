//! Plan GI3: a relayed `RequestSegmentExpansion` (Plate Gate, architecture
//! §15.1–§15.2), through the one Host ingest both modes call (the daemon's
//! `ingest_gossip`, the desktop's relay dispatch).
//!
//! A joiner that finds the highest segment full and lacks MANAGE_COMMUNITY
//! gossips the request (`join_stages`); any member that holds
//! MANAGE_COMMUNITY under the merged governance answers by expanding the
//! community one segment. The first expansion wins through the merge: a
//! later admin's `highest_segment_full` pre-flight sees the new segment and
//! the call is refused, so concurrent answers are harmless.

use rekindle_codec::community::envelope::ControlPayload;
use rekindle_governance_runtime as gov_rt;

use super::HostGovernance;

impl HostGovernance {
    /// Answer a relayed segment-expansion request: `None` when it is
    /// refused (not that payload, a request naming another community than
    /// the envelope's, or we do not hold MANAGE_COMMUNITY), else the
    /// expansion's outcome (the new segment's index, or why it did not run).
    pub async fn on_segment_expansion_request(
        &self,
        community_id: &str,
        payload: &ControlPayload,
    ) -> Option<Result<u32, String>> {
        let ControlPayload::RequestSegmentExpansion {
            community_id: requested,
            requester_pseudonym,
            full_segment_index,
        } = payload
        else {
            return None;
        };
        // A forged envelope cannot redirect us to another community.
        if requested != community_id {
            tracing::debug!(community = %community_id, requested = %requested, "segment expansion request for another community dropped");
            return None;
        }
        if !self.we_manage_community(community_id) {
            tracing::debug!(community = %community_id, requester = %requester_pseudonym, full_segment_index, "segment expansion request ignored: we lack MANAGE_COMMUNITY");
            return None;
        }
        tracing::info!(community = %community_id, requester = %requester_pseudonym, full_segment_index, "segment expansion requested — expanding");
        let outcome = gov_rt::expand_community_segment(self, community_id)
            .await
            .map_err(|e| e.to_string());
        if let Err(error) = &outcome {
            // A concurrent admin's expansion already landed, or the cap.
            tracing::debug!(community = %community_id, %error, "segment expansion did not run");
        }
        Some(outcome)
    }

    /// Whether our own pseudonym holds MANAGE_COMMUNITY under the merged
    /// governance (the authority `expand_community_segment` needs).
    fn we_manage_community(&self, community_id: &str) -> bool {
        let Some(me) = gov_rt::GovernanceRuntimeDeps::my_pseudonym(self, community_id) else {
            return false;
        };
        self.ctx
            .communities
            .governance_state(community_id)
            .is_some_and(|state| {
                rekindle_types::permissions::Permissions(
                    rekindle_governance::permissions::compute_permissions(&me, None, &state, 0),
                )
                .has(rekindle_types::permissions::MANAGE_COMMUNITY)
            })
    }
}

#[cfg(test)]
#[path = "segment_request_tests.rs"]
mod tests;
