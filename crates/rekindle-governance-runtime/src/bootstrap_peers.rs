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
//! state names every member with its member record. It asks only members
//! whose beat (`M`/1) is fresh, unexpired and not `Offline`, in the order
//! the architecture gives a requester its responder
//! (`.claude/docs/rekindle-communities-architecture.md` §7.1,
//! `select_mek_responder`: members with recent heartbeats, lowest
//! `blake3(requester ‖ member)` first; the same rank,
//! `rekindle_secrets::rotator::cascade_candidates`, with us as the
//! context). No member stands above another (ADR 0011), so the load of
//! joins spreads over the live members instead of falling on the founders,
//! and a member that is offline costs one beat read, not a call timeout.
//! Each asked member's current route is on its `M`/6, signed by its
//! pseudonym ([`crate::member_routes::read_member_route`]).
//!
//! **How many, how long.** At most [`MAX_BOOTSTRAP_PEERS`] live members
//! besides the inviter, within [`bootstrap_peers_cap`] in all. The exchange
//! is an accelerator, never the join's authority: a joiner can always
//! rebuild the member set from the DHT and joining works with nobody online
//! (ADR 0011), and any member that merges our `MemberJoined` delivers the
//! community key (D16). So past the cap the join stands and the DHT rebuild
//! proceeds. The cap is one Veilid answer wait per member asked
//! (`rekindle_types::config::answer_wait`: 6 s at our 3-hop floor,
//! veilid-core 0.5.8 `rpc_processor/mod.rs:109-116,1993`), 24 s.
//!
//! **Refusals.** Only the inviter's NAK withdraws the join: it minted the
//! invite, so its merge always holds the invite op our `MemberJoined`
//! cites. Another member may not have merged that op yet and would refuse
//! what every member will accept once it has, so its NAK, like its
//! silence, moves on to the next member.

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance::state::GovernanceState;
use rekindle_records::port::RecordPort;
use rekindle_types::config::{
    answer_wait, SafetyProfile, SequencingPreference, StabilityPreference, ANONYMITY_HOP_FLOOR,
};
use rekindle_types::governance::OpRef;
use rekindle_types::id::PseudonymKey;

use crate::error::GovernanceRuntimeError;
use crate::member_card::{beat_is_live, read_beat, MemberRecordRef};

/// How many live members besides the inviter a joiner asks (see the
/// module docs).
pub const MAX_BOOTSTRAP_PEERS: usize = 4;

/// The total time a joiner spends asking members after a partial inviter:
/// one Veilid answer wait at our hop floor per member it may ask (see the
/// module docs).
#[must_use]
pub fn bootstrap_peers_cap() -> Duration {
    let floor = SafetyProfile {
        hop_count: ANONYMITY_HOP_FLOOR,
        stability: StabilityPreference::default(),
        sequencing: SequencingPreference::default(),
    };
    answer_wait(&floor) * u32::try_from(MAX_BOOTSTRAP_PEERS).unwrap_or(u32::MAX)
}

/// A member a joiner may ask: its pseudonym, its member record and that
/// record's owner (as its `MemberJoined` names them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapPeer {
    pub pseudonym: PseudonymKey,
    pub record: String,
    pub owner: String,
}

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
/// departed or banned, neither us (`joiner`) nor the inviter
/// (`inviter_record`), in `select_mek_responder`'s order with us as the
/// requester (lowest `blake3(joiner ‖ member)` first). Whose beat is
/// fresh is read when each is reached ([`find_frontier`]).
#[must_use]
pub fn candidates(
    state: &GovernanceState,
    joiner: &PseudonymKey,
    inviter_record: &str,
) -> Vec<BootstrapPeer> {
    let eligible: Vec<PseudonymKey> = (state.members.iter())
        .filter(|(key, m)| {
            *key != joiner && m.member_record != inviter_record && !state.is_departed(key)
        })
        .map(|(key, _)| key.clone())
        .collect();
    rekindle_secrets::rotator::cascade_candidates(joiner, &eligible, eligible.len())
        .into_iter()
        .filter_map(|key| {
            let m = state.members.get(&key)?;
            Some(BootstrapPeer {
                record: m.member_record.clone(),
                owner: m.member_record_owner.clone(),
                pseudonym: key,
            })
        })
        .collect()
}

/// Whether `candidate`'s verified beat is fresh at `now_ms`: present
/// (online, away or busy) and live by the 15-minute rule
/// ([`beat_is_live`]). The beat is read cold, so only its beat number
/// counts (plan E3.4 S1, Q4).
async fn beat_is_fresh(
    port: &dyn RecordPort,
    anchor: &str,
    candidate: &BootstrapPeer,
    now_ms: u64,
) -> bool {
    let member = MemberRecordRef {
        community_id: anchor,
        pseudonym: &candidate.pseudonym,
        record: &candidate.record,
        owner: &candidate.owner,
    };
    match read_beat(port, &member).await {
        Ok(Some(beat)) => beat.state.is_present() && beat_is_live(beat.beat_number, None, now_ms),
        Ok(None) => false,
        Err(error) => {
            tracing::debug!(community = anchor, %error, "a member's beat was refused");
            false
        }
    }
}

/// Ask `candidates` in order, skipping those whose beat is stale, until
/// [`MAX_BOOTSTRAP_PEERS`] were asked: the first frontier answered.
/// `asked` counts the members asked.
async fn ask_members<F, Fut>(
    call: &F,
    port: &dyn RecordPort,
    (anchor, now_ms): (&str, u64),
    candidates: &[BootstrapPeer],
    request: &[u8],
    asked: &AtomicUsize,
) -> Option<Vec<OpRef>>
where
    F: Fn(Vec<u8>, Vec<u8>) -> Fut,
    Fut: Future<Output = Result<Vec<u8>, GovernanceRuntimeError>>,
{
    for candidate in candidates {
        if asked.load(Ordering::Relaxed) >= MAX_BOOTSTRAP_PEERS {
            break;
        }
        if !beat_is_fresh(port, anchor, candidate, now_ms).await {
            continue;
        }
        let (member, record) = (&candidate.pseudonym, &candidate.record);
        let route =
            match crate::member_routes::read_member_route(port, anchor, record, member).await {
                Ok(Some(route)) => route,
                Ok(None) => continue,
                Err(error) => {
                    tracing::info!(community = anchor, %error, "a member's route was refused");
                    continue;
                }
            };
        asked.fetch_add(1, Ordering::Relaxed);
        let answer = match call(route, request.to_vec()).await {
            Ok(reply) => read_reply(&reply, anchor),
            Err(error) => Err(error),
        };
        match answer {
            Ok(BootstrapReply::Frontier(frontier)) => return Some(frontier),
            Ok(BootstrapReply::Partial) => {}
            Err(error) => {
                tracing::info!(community = anchor, %error, "a member did not answer the bootstrap");
            }
        }
    }
    None
}

/// Send `request` to the inviter at `inviter_route` and, when it answers
/// `bootstrapPartial`, to the live members among `candidates` in order
/// through their `M`/6 routes, within [`bootstrap_peers_cap`]: the first
/// frontier answered. `now_ms` judges each beat.
///
/// # Errors
/// The inviter's call failed (it is unreachable: the join stands from the
/// DHT) or it refused ([`GovernanceRuntimeError::BootstrapRefused`]), or
/// the inviter was partial and no member answered a frontier within the
/// cap ([`GovernanceRuntimeError::BootstrapUnavailable`], with the number
/// asked).
pub async fn find_frontier<F, Fut>(
    call: F,
    port: &dyn RecordPort,
    (anchor, now_ms): (&str, u64),
    inviter_route: &[u8],
    candidates: &[BootstrapPeer],
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
    let asked = AtomicUsize::new(0);
    let members = ask_members(&call, port, (anchor, now_ms), candidates, request, &asked);
    match tokio::time::timeout(bootstrap_peers_cap(), members).await {
        Ok(Some(frontier)) => Ok(frontier),
        Ok(None) => Err(GovernanceRuntimeError::BootstrapUnavailable(
            asked.load(Ordering::Relaxed),
        )),
        Err(_) => {
            tracing::info!(
                community = anchor,
                "the bootstrap cap passed: the DHT rebuild stands"
            );
            Err(GovernanceRuntimeError::BootstrapUnavailable(
                asked.load(Ordering::Relaxed),
            ))
        }
    }
}

#[cfg(test)]
#[path = "bootstrap_peers_tests.rs"]
mod tests;
