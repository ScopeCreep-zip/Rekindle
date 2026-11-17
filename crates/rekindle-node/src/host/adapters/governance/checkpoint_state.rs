//! The checkpoint state snapshot a member serves (plan E3.2 S7c.3, "State
//! snapshot"), on both modes: answered only to an admitted member and
//! rate-limited per requester like the `headAdvanced` handler, in chunks of
//! at most 30,000 B (`rekindle_governance_runtime::checkpoint_state`).

use rekindle_codec::checkpoint_state::decode_checkpoint_state_request;
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;

impl HostGovernance {
    /// Answer a `CheckpointStateRequest` (the encoded request its signer's
    /// pseudonym signs): the encoded `checkpointStateResponse` envelope, or
    /// `None` (a NAK) when it does not verify, its signer is not a member,
    /// the signer is over its rate, or we hold no such snapshot.
    pub async fn answer_checkpoint_state(&self, request: &[u8]) -> Option<Vec<u8>> {
        let request = match decode_checkpoint_state_request(request) {
            Ok(request) => request,
            Err(error) => {
                tracing::debug!(%error, "checkpoint state request did not verify");
                return None;
            }
        };
        let community = request.community.as_str();
        let requester = PseudonymKey(request.requester);
        if !self.is_key_reader(community, &requester) {
            tracing::debug!(
                community,
                "checkpoint state request from a non-member refused"
            );
            return None;
        }
        if !self.admit_snapshot_request(community, &requester) {
            tracing::debug!(
                community,
                "checkpoint state request over the requester's rate"
            );
            return None;
        }
        match rekindle_governance_runtime::checkpoint_state::answer_state_request(self, &request)
            .await
        {
            Ok(reply) => Some(reply),
            Err(error) => {
                tracing::debug!(community, %error, "checkpoint state not served");
                None
            }
        }
    }

    /// One token of `requester`'s bucket in `community`, at the 10 per
    /// second floor the gossip ingest meters a `headAdvanced` sender at
    /// (`rekindle_transport::gossip::RateLimiter`).
    fn admit_snapshot_request(&self, community: &str, requester: &PseudonymKey) -> bool {
        let mut map = self.ctx.communities.map().write();
        map.get_mut(community)
            .is_some_and(|c| c.snapshot_requests.check(&hex::encode(requester.0)))
    }
}
