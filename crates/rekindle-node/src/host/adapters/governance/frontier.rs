//! The frontier a member serves (plan E3.2 S7d-keys.8, the unlock
//! barrier), on both modes: answered by the Host governance adapter only to
//! a requester whose signature names an admitted member (reader-validated
//! against our governance state: a member, not banned), rate-limited per
//! requester at the `headAdvanced` handler's floor, with our own `Frontier`
//! (`rekindle_governance_runtime::frontier_exchange`).

use rekindle_codec::frontier_request::decode_frontier_request;
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;

impl HostGovernance {
    /// Answer a `FrontierRequest` (the encoded request its signer's
    /// pseudonym signs): the encoded `frontierResponse` envelope, or `None`
    /// (a NAK, no frontier) when it does not verify, its signer is not an
    /// admitted member, the asker is over its rate, or our frontier could
    /// not be built.
    pub async fn answer_frontier(&self, request: &[u8]) -> Option<Vec<u8>> {
        let request = match decode_frontier_request(request) {
            Ok(request) => request,
            Err(error) => {
                tracing::debug!(%error, "frontier request did not verify");
                return None;
            }
        };
        let community = request.community.as_str();
        let requester = PseudonymKey(request.requester);
        if !self.is_key_reader(community, &requester) {
            tracing::debug!(community, "frontier request from a non-member refused");
            return None;
        }
        let bucket = format!("frontier:{}", hex::encode(requester.0));
        if !self.admit_snapshot_request(community, &bucket) {
            tracing::debug!(community, "frontier request over the asker's rate");
            return None;
        }
        match rekindle_governance_runtime::frontier_exchange::answer_frontier_request(
            self, &request,
        )
        .await
        {
            Ok(reply) => Some(reply),
            Err(error) => {
                tracing::debug!(community, %error, "frontier not served");
                None
            }
        }
    }
}
