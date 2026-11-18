//! Whom a joiner asks for the bootstrap frontier (plan E3.2 S7c.8 (i)).
//!
//! An inviter that joined from a checkpoint hint and has not backfilled is
//! partial (`crate::partial`): it answers a `BootstrapRequest` with
//! `bootstrapPartial`, never a frontier it cannot vouch for and never a
//! refusal, and the joiner asks another member. This is Synapse's shape: a
//! partial-state resident answers `UNABLE_DUE_TO_PARTIAL_STATE`
//! (`event_auth.py` `assert_host_in_room`), an error code the joining
//! server's `FederationClient._try_destination_list` fails over on, moving
//! to the next resident of `get_current_hosts_in_room_ordered`.
//!
//! **Whom.** Before its bootstrap the joiner already holds the community's
//! roster: its walk (or hinted snapshot) and rebuild ran, so the merged
//! state names every member with its member record. Each member's current
//! route is on its `M`/6, signed by its pseudonym
//! ([`crate::member_routes::read_member_route`]), readable by anyone who
//! knows the record. Members are asked oldest join first, as Synapse
//! orders residents by their oldest join: an older member is the one most
//! likely to hold the whole op set rather than be partial itself.
//!
//! **How many.** At most [`MAX_BOOTSTRAP_PEERS`] besides the inviter. The
//! exchange is a speed-up, never the join's authority: the join already
//! stands from the DHT (ADR 0011), and any member that merges our
//! `MemberJoined` delivers the community key (D16). Each attempt costs a
//! DHT read of `M`/6 and an `app_call` that can run to its timeout, so an
//! unbounded loop over a large roster would hold the join command for
//! minutes; four bounds the worst case to a handful of call timeouts.
//!
//! **Refusals.** Only the inviter's NAK withdraws the join: it minted the
//! invite, so its merge always holds the invite op our `MemberJoined`
//! cites. Another member may not have merged that op yet and would refuse
//! what every member will accept once it has, so its NAK, like its
//! silence, moves on to the next member.

use std::future::Future;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance::state::GovernanceState;
use rekindle_records::port::RecordPort;
use rekindle_types::governance::OpRef;
use rekindle_types::id::PseudonymKey;

use crate::error::GovernanceRuntimeError;

/// How many members besides the inviter a joiner asks (see the module
/// docs).
pub const MAX_BOOTSTRAP_PEERS: usize = 4;

/// A responder's answer to our `BootstrapRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapReply {
    /// Where our walk continues.
    Frontier(Vec<OpRef>),
    /// The responder is partial: ask another member.
    Partial,
}

/// Read a responder's reply for `anchor`.
///
/// # Errors
/// [`GovernanceRuntimeError::BootstrapRefused`]: the reply is neither a
/// `bootstrapResponseV2` nor a `bootstrapPartial` for `anchor` (a NAK).
pub fn read_reply(reply: &[u8], anchor: &str) -> Result<BootstrapReply, GovernanceRuntimeError> {
    match rekindle_codec::capnp_envelope::try_decode_community_envelope(reply) {
        Ok(Some(CommunityEnvelope::Control(ControlPayload::BootstrapResponseV2 {
            anchor: answered,
            frontier,
        }))) if answered == anchor => Ok(BootstrapReply::Frontier(frontier)),
        Ok(Some(CommunityEnvelope::Control(ControlPayload::BootstrapPartial {
            anchor: answered,
        }))) if answered == anchor => Ok(BootstrapReply::Partial),
        _ => Err(GovernanceRuntimeError::BootstrapRefused(
            "the reply is not a bootstrapResponseV2 for this community".into(),
        )),
    }
}

/// The members to ask after a partial inviter: `state`'s members, not
/// banned, neither us nor the inviter (`inviter_record`), oldest join
/// first, at most [`MAX_BOOTSTRAP_PEERS`]; each with its member record.
#[must_use]
pub fn candidates(
    state: &GovernanceState,
    joiner: &PseudonymKey,
    inviter_record: &str,
) -> Vec<(PseudonymKey, String)> {
    let mut members: Vec<_> = state
        .members
        .iter()
        .filter(|(key, m)| {
            *key != joiner && m.member_record != inviter_record && !state.bans.contains(*key)
        })
        .collect();
    members.sort_by(|(a, ma), (b, mb)| ma.joined.cmp(&mb.joined).then_with(|| a.0.cmp(&b.0)));
    members
        .into_iter()
        .take(MAX_BOOTSTRAP_PEERS)
        .map(|(key, m)| (key.clone(), m.member_record.clone()))
        .collect()
}

/// Send `request` to the inviter at `inviter_route` and, when it answers
/// `bootstrapPartial`, to each of `candidates` in order through its `M`/6
/// route: the first frontier answered.
///
/// # Errors
/// The inviter's call failed (it is unreachable: the join stands from the
/// DHT) or it refused ([`GovernanceRuntimeError::BootstrapRefused`]), or
/// the inviter was partial and no candidate answered a frontier
/// ([`GovernanceRuntimeError::BootstrapUnavailable`]).
pub async fn find_frontier<F, Fut>(
    call: F,
    port: &dyn RecordPort,
    anchor: &str,
    inviter_route: &[u8],
    candidates: &[(PseudonymKey, String)],
    request: &[u8],
) -> Result<Vec<OpRef>, GovernanceRuntimeError>
where
    F: Fn(Vec<u8>, Vec<u8>) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, GovernanceRuntimeError>>,
{
    let reply = call(inviter_route.to_vec(), request.to_vec()).await?;
    if let BootstrapReply::Frontier(frontier) = read_reply(&reply, anchor)? {
        return Ok(frontier);
    }
    tracing::info!(
        community = anchor,
        "the inviter is partial: asking other members"
    );
    for (member, record) in candidates {
        let route =
            match crate::member_routes::read_member_route(port, anchor, record, member).await {
                Ok(Some(route)) => route,
                Ok(None) => continue,
                Err(error) => {
                    tracing::info!(community = anchor, %error, "a member's route was refused");
                    continue;
                }
            };
        let answer = match call(route, request.to_vec()).await {
            Ok(reply) => read_reply(&reply, anchor),
            Err(error) => Err(error),
        };
        match answer {
            Ok(BootstrapReply::Frontier(frontier)) => return Ok(frontier),
            Ok(BootstrapReply::Partial) => {}
            Err(error) => {
                tracing::info!(community = anchor, %error, "a member did not answer the bootstrap");
            }
        }
    }
    Err(GovernanceRuntimeError::BootstrapUnavailable(
        candidates.len(),
    ))
}

#[cfg(test)]
#[path = "bootstrap_peers_tests.rs"]
mod tests;
