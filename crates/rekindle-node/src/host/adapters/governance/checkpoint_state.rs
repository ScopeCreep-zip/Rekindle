//! The checkpoint state snapshot a member serves (plan E3.2 S7c.3, "State
//! snapshot"), on both modes: answered only to an admitted member, or to a
//! joiner holding a live invite (plan E3.2 S7c.5, the pre-join hint), and
//! rate-limited per requester like the `headAdvanced` handler, in chunks of
//! at most 30,000 B (`rekindle_governance_runtime::checkpoint_state`).

use rekindle_codec::checkpoint_hint::decode_checkpoint_hint_request;
use rekindle_codec::checkpoint_state::decode_checkpoint_state_request;
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;

impl HostGovernance {
    /// Answer a `CheckpointStateRequest` (the encoded request its signer's
    /// pseudonym signs): the encoded `checkpointStateResponse` envelope, or
    /// `None` (a NAK) when it does not verify, its signer is not a member
    /// (a member's request) or holds no live invite (a joiner's), the
    /// signer is over its rate, or we hold no such snapshot.
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
        let invite = request.invite_code_hash.as_deref();
        if !self.admit_reader(community, &requester, invite) {
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

    /// Answer a `CheckpointHintRequest` (plan E3.2 S7c.5): the encoded
    /// `checkpointHintResponse` envelope, or `None` (a NAK) when it does
    /// not verify, its signer is neither a member nor a joiner holding a
    /// live invite, or the signer is over its rate.
    pub async fn answer_checkpoint_hint(&self, request: &[u8]) -> Option<Vec<u8>> {
        let request = match decode_checkpoint_hint_request(request) {
            Ok(request) => request,
            Err(error) => {
                tracing::debug!(%error, "checkpoint hint request did not verify");
                return None;
            }
        };
        let community = request.community.as_str();
        let requester = PseudonymKey(request.requester);
        let invite = request.invite_code_hash.as_deref();
        if !self.admit_reader(community, &requester, invite) {
            return None;
        }
        match rekindle_governance_runtime::checkpoint_hint::answer_hint_request(self, &request)
            .await
        {
            Ok(reply) => Some(reply),
            Err(error) => {
                tracing::debug!(community, %error, "checkpoint hint not served");
                None
            }
        }
    }

    /// Whether `requester` may read `community`'s checkpoint hint or
    /// snapshot now: a member (no `invite`) or a joiner holding the live
    /// invite `invite` names, within its rate.
    fn admit_reader(
        &self,
        community: &str,
        requester: &PseudonymKey,
        invite: Option<&str>,
    ) -> bool {
        let admitted = match invite {
            None => self.is_key_reader(community, requester),
            Some(code_hash) => self.holds_live_invite(community, requester, code_hash),
        };
        if !admitted {
            tracing::debug!(
                community,
                "checkpoint request from a non-member without a live invite refused"
            );
            return false;
        }
        if !self.admit_snapshot_request(community, requester) {
            tracing::debug!(community, "checkpoint request over the requester's rate");
            return false;
        }
        true
    }

    /// Whether `requester`, not banned, names a live invite of `community`
    /// (`GovernanceState::live_invite`, the check the join itself passes:
    /// a revoked, expired or used-up invite is refused) and has a place in
    /// that invite's pre-join budget (plan E3.2 S7c.8 (2)): at most
    /// `max_uses` distinct requesters per code hash, a requester admitted
    /// once keeping its place. Each honest joiner needs one place and an
    /// invite admits `max_uses` joins, so an honest invite never runs out;
    /// whoever saw its `InviteCreated` and asks without joining spends a
    /// place (threat model I16), and a joiner refused here walks in full.
    pub(crate) fn holds_live_invite(
        &self,
        community: &str,
        requester: &PseudonymKey,
        code_hash: &str,
    ) -> bool {
        let now = self.ctx.clock.now_secs();
        let Some(state) = self.ctx.communities.governance_state(community) else {
            return false;
        };
        if state.bans.contains(requester) {
            return false;
        }
        let Some(budget) = state.live_invite(code_hash, now).map(|i| i.max_uses) else {
            return false;
        };
        let mut map = self.ctx.communities.map().write();
        map.get_mut(community).is_some_and(|c| {
            let readers = c.pre_join_readers.entry(code_hash.to_owned()).or_default();
            if readers.contains(&requester.0) {
                return true;
            }
            if readers.len() >= budget as usize {
                return false;
            }
            readers.insert(requester.0);
            true
        })
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
