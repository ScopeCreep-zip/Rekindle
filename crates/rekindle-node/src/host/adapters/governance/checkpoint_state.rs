//! The checkpoint state snapshot a member serves (plan E3.2 S7c.3, "State
//! snapshot"), on both modes: answered only to an admitted member, or to a
//! joiner holding a live invite of ours (plan E3.2 S7c.5, the pre-join
//! hint), its request tagged under the invite code (S7c.8 (2)), and
//! rate-limited like the `headAdvanced` handler, in chunks of at most
//! 30,000 B (`rekindle_governance_runtime::checkpoint_state`).

use rekindle_codec::checkpoint_hint::decode_checkpoint_hint_request;
use rekindle_codec::checkpoint_state::{decode_checkpoint_state_request, PreJoinRequest};
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;

impl HostGovernance {
    /// Answer a `CheckpointStateRequest` (the encoded request its signer's
    /// pseudonym signs): the encoded `checkpointStateResponse` envelope, or
    /// `None` (a NAK) when it does not verify, its signer is not a member
    /// (a member's request) or does not prove a live invite of ours (a
    /// joiner's), the asker is over its rate, or we hold no such snapshot.
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
        let invite = request.invite.as_ref().map(|i| i.code_hash.as_str());
        let proves = |code: &str| request.invite_proven_by(code);
        if !self
            .admit_reader(community, &requester, invite, proves)
            .await
        {
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
    /// not verify, its signer is neither a member nor a joiner proving a
    /// live invite of ours, or the asker is over its rate.
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
        let invite = request.invite.as_ref().map(|i| i.code_hash.as_str());
        let proves = |code: &str| request.invite_proven_by(code);
        if !self
            .admit_reader(community, &requester, invite, proves)
            .await
        {
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
    /// snapshot now: a member (no `invite`), or a joiner whose request
    /// names the code hash `invite` of a live invite we minted and is
    /// tagged under its code (`proves`, plan E3.2 S7c.8 (2)), within the
    /// rate. A Veilid `app_call` carries no sender (veilid-core 0.5.8
    /// `rpc_processor/rpc_app_call.rs:102-114`) and a joiner's pseudonym is
    /// its own choice, so a joiner is metered per invite, a member per
    /// pseudonym.
    async fn admit_reader(
        &self,
        community: &str,
        requester: &PseudonymKey,
        invite: Option<&str>,
        proves: impl Fn(&str) -> bool,
    ) -> bool {
        let (admitted, bucket) = match invite {
            None => (
                self.is_key_reader(community, requester),
                hex::encode(requester.0),
            ),
            Some(code_hash) => {
                let proven = self
                    .our_invite_code(community, code_hash)
                    .await
                    .is_some_and(|code| proves(&code));
                (
                    proven && self.holds_live_invite(community, requester, code_hash),
                    format!("invite:{code_hash}"),
                )
            }
        };
        if !admitted {
            tracing::debug!(
                community,
                "checkpoint request from a non-member without a live invite refused"
            );
            return false;
        }
        if !self.admit_snapshot_request(community, &bucket) {
            tracing::debug!(community, "checkpoint request over the asker's rate");
            return false;
        }
        true
    }

    /// The code of our invite of `community` whose code hash is
    /// `code_hash`, `None` when it is not ours (only its minter holds the
    /// code: the op log carries the hash) or the read failed.
    async fn our_invite_code(&self, community: &str, code_hash: &str) -> Option<String> {
        let owner = self.ctx.require_owner().ok()?;
        let db = self.ctx.db.current().ok()?;
        let (cid, hash) = (community.to_owned(), code_hash.to_owned());
        db.call(move |c| rekindle_db::repo::community_invites::code(c, &owner, &cid, &hash))
            .await
            .inspect_err(|error| tracing::debug!(%error, "our invite code"))
            .ok()
            .flatten()
    }

    /// Whether `requester`, not banned, names a live invite of `community`
    /// (`GovernanceState::live_invite`, the check the join itself passes:
    /// a revoked, expired or used-up invite is refused). Only a holder of
    /// the code reaches here (`admit_reader`), and the code already lets it
    /// join and read what a member reads, so a pre-join read discloses
    /// nothing the invite does not grant (threat model I16).
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
        !state.bans.contains(requester) && state.live_invite(code_hash, now).is_some()
    }

    /// One token of `bucket`'s rate in `community`, at the 10 per second
    /// floor the gossip ingest meters a `headAdvanced` sender at
    /// (`rekindle_transport::gossip::RateLimiter`).
    fn admit_snapshot_request(&self, community: &str, bucket: &str) -> bool {
        let mut map = self.ctx.communities.map().write();
        map.get_mut(community)
            .is_some_and(|c| c.snapshot_requests.check(bucket))
    }
}
