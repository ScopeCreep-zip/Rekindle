//! The bootstrap exchange of the self-sovereign join (plan E3.2, ADR 0011).
//!
//! The join works from the DHT with nobody online; when the inviter is
//! online, this exchange is how it learns of the joiner. The joiner sends
//! `BootstrapRequest{joinerPseudonym, anchor, memberJoined}` as a bare
//! `app_call` to the inviter's current route, read from its `M`/6 by the
//! join (`crate::member_routes::read_member_route`), once its `MemberJoined`
//! is durable on its member record. The request's authority is that
//! signed op, carried with its `OpRef` (an inviter answers within the
//! app_call reply window, too short for a DHT read): the inviter checks
//! it against the `OpRef`, verifies it and merges it in memory first: an
//! op the merge does not admit (no member and no pending knock results) is
//! never stored, since the request names no sender and its op lives in the
//! joiner's own record, where any reader fetches it; an admitted op is
//! stored and merged, and only a joiner who is then a member of the
//! merged state is answered, with `bootstrapResponseV2{anchor, frontier}`:
//! where the joiner's walk of the op DAG continues. It ships no keys (D11;
//! the joiner asks for the community key by `RequestMEK`), no messages and
//! no checkpoint hint (the joiner asks for that before its walk, by
//! `checkpointHintRequest`).

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_types::governance::OpRef;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::event::GovernanceRuntimeEvent;
use crate::op_log::{self, HeldOp};

/// The `app_call` reply cap a response stays under (Veilid's
/// `MAX_APP_CALL_Q_MESSAGE_LEN`).
pub const MAX_BOOTSTRAP_RESPONSE_LEN: usize = 32_768;

/// Who asks: the joiner's pseudonym and its `MemberJoined` (op #0, or a
/// re-join's later op).
#[derive(Debug, Clone)]
pub struct BootstrapJoiner {
    /// Hex of the joiner's community pseudonym.
    pub pseudonym: String,
    pub member_joined: OpRef,
    /// The encoded signed op `member_joined` names.
    pub member_joined_op: Vec<u8>,
    /// The joiner's route, where the inviter delivers the community key
    /// (D16).
    pub route: Vec<u8>,
}

fn adapter(msg: impl Into<String>) -> GovernanceRuntimeError {
    GovernanceRuntimeError::Adapter(msg.into())
}

/// Our encoded answer to a `BootstrapRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapAnswer {
    /// `bootstrapResponseV2`: we merged the joiner's `MemberJoined`, and
    /// the caller delivers it the community key (D16).
    Frontier(Vec<u8>),
    /// `bootstrapPartial` (S7c.8 (i)): we joined from a hint and have not
    /// backfilled, so we vouch for no frontier and merged nothing; the
    /// joiner asks another member (`crate::bootstrap_peers`).
    Partial(Vec<u8>),
    /// `bootstrapRefused` (plan E3.2 join step 5, IJ2): our merge does not
    /// admit the joiner's `MemberJoined`, for the typed reason it names
    /// (`JoinRefusal`). The answer itself, so every host forwards it as is
    /// and none can drop the reason to a bare NAK.
    Refused(Vec<u8>),
    /// `RETRY_LATER` (`rekindle_types::message::CALL_RETRY_LATER`, plan
    /// E3.2 JN1): the joiner's `MemberJoined` waits on a dep we hold not
    /// and could not fetch yet, so we cannot judge it (BR1's vocabulary: a
    /// call we cannot judge is no refusal). We merged and stored nothing;
    /// the joiner retries (`bootstrap_peers`: `PeerRetryLater`). The answer
    /// itself, so every host forwards it as is, never a bare NAK.
    RetryLater(Vec<u8>),
}

/// Answer a joiner's `BootstrapRequest`: while we are partial, with
/// `bootstrapPartial`; otherwise check its `MemberJoined` against the
/// `OpRef`, verify it, merge it in memory, store it only when that merge
/// admits it, then encode `bootstrapResponseV2`
/// with our frontier.
///
/// # Errors
/// We are not in the community, the op is not the joiner's `MemberJoined`
/// or does not match its `OpRef`, or the store or the encode failed. The
/// caller NAKs. A join the merge does not admit is answered
/// [`BootstrapAnswer::Refused`], one we cannot judge yet (a dep not
/// fetchable) [`BootstrapAnswer::RetryLater`].
pub async fn answer_bootstrap_request<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    joiner: &BootstrapJoiner,
) -> Result<BootstrapAnswer, GovernanceRuntimeError> {
    if crate::partial::is_partial(deps, anchor).await {
        let payload = ControlPayload::BootstrapPartial {
            anchor: anchor.to_owned(),
        };
        return encode_response(payload).map(BootstrapAnswer::Partial);
    }
    match build_bootstrap_response_v2(deps, anchor, joiner).await {
        Ok(bytes) => Ok(BootstrapAnswer::Frontier(bytes)),
        Err(GovernanceRuntimeError::JoinNotAdmitted(reason)) => {
            tracing::warn!(
                community = anchor,
                %reason,
                "bootstrap request refused: the merge does not admit the joiner"
            );
            encode_refusal(anchor, reason).map(BootstrapAnswer::Refused)
        }
        Err(GovernanceRuntimeError::JoinUnresolved { missing }) => {
            tracing::info!(
                community = anchor,
                missing,
                "bootstrap request unresolved: a dep of the joiner's op is not fetchable yet; answered retry-later"
            );
            Ok(BootstrapAnswer::RetryLater(
                rekindle_types::message::CALL_RETRY_LATER.to_vec(),
            ))
        }
        Err(e) => Err(e),
    }
}

async fn build_bootstrap_response_v2<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    joiner: &BootstrapJoiner,
) -> Result<Vec<u8>, GovernanceRuntimeError> {
    deps.community_membership(anchor)
        .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(anchor.to_owned()))?;
    let author = &joiner.member_joined.author;
    if hex::encode(author.0) != joiner.pseudonym {
        return Err(adapter("bootstrap: the OpRef is not the joiner's"));
    }
    let op = op_log::held_from_encoded(&joiner.member_joined, joiner.member_joined_op.clone())
        .ok_or_else(|| adapter("bootstrap: the op does not match the joiner's OpRef"))?;
    let before = deps.governance_state(anchor).unwrap_or_default();
    if !is_joiners_member_joined(&op, &joiner.member_joined, &before) {
        return Err(adapter(
            "bootstrap: the op is not the joiner's MemberJoined (op #0, or a re-join after its departure)",
        ));
    }
    // Check, store and (on the last use) revoke under one lock (plan SU1):
    // two joiners on one invite are admitted one after the other.
    let admission = deps.admission_lock(anchor);
    let admitting = admission.lock().await;
    for dep in admitted_deps(deps, anchor, &op).await? {
        deps.op_store().store_governance_op(anchor, &dep).await?;
    }
    let joined_op = op.clone();
    op_log::receive_op(deps, anchor, op.encoded).await?;
    let state = deps
        .governance_state(anchor)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(anchor.to_owned()))?;
    if !holds_joined(&state, author) {
        return Err(GovernanceRuntimeError::JoinNotAdmitted(
            rekindle_types::join_refusal::JoinRefusal::NotAdmitted,
        ));
    }
    revoke_if_used_up(deps, anchor, &joined_op, &state).await?;
    drop(admitting);
    let state = deps
        .governance_state(anchor)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(anchor.to_owned()))?;
    let cert = deps
        .genesis_cert(anchor)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(anchor.to_owned()))?;
    let set =
        op_log::verify_held_with(deps, anchor, &deps.op_store().governance_ops(anchor).await?);
    let payload = ControlPayload::BootstrapResponseV2 {
        anchor: anchor.to_owned(),
        frontier: op_log::frontier_refs(&cert, &state, &set),
    };
    let bytes = encode_response(payload)?;
    deps.emit_event(GovernanceRuntimeEvent::BootstrapResponseBuilt {
        community_id: anchor.to_owned(),
        joiner_pseudonym_hex: joiner.pseudonym.clone(),
        bytes: bytes.len(),
    });
    Ok(bytes)
}

/// The deps `op` cites that we do not hold, when the merge of our op set,
/// those deps (fetched by their `OpRef` and checked against its hash, as a
/// rebuild fetches them) and `op`, all in memory, applies `op` and holds
/// its author as a member or a pending one (ADR 0011: an
/// `ApprovalRequired` join is a knock readers keep). Nothing is stored
/// before this answers.
///
/// The merge judges `op` only once its whole causal past is held: when a
/// dep it waits on (transitively) is neither held nor fetchable yet, it
/// answers [`GovernanceRuntimeError::JoinUnresolved`] (plan E3.2 JN1), not
/// a refusal, since a merge missing the op's past cannot apply it whatever
/// its merit.
///
/// # Errors
/// `op` waits on an unheld dep ([`GovernanceRuntimeError::JoinUnresolved`]:
/// the caller answers retry later and stores nothing), the merge does not
/// admit `op` (the caller refuses and stores nothing), no cert is pinned,
/// or the op store failed. A dep fetch that fails leaves that dep unheld.
async fn admitted_deps<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    op: &HeldOp,
) -> Result<Vec<HeldOp>, GovernanceRuntimeError> {
    let cert = deps
        .genesis_cert(anchor)
        .ok_or_else(|| GovernanceRuntimeError::GenesisCertMissing(anchor.to_owned()))?;
    let mut held = deps.op_store().governance_ops(anchor).await?;
    held.push(op.clone());
    let logs = op_log::WalkLogCache::default();
    let mut tried = std::collections::BTreeSet::new();
    let mut fetched = Vec::new();
    loop {
        let set = op_log::verify_held_with(deps, anchor, &held);
        let wanted: Vec<OpRef> = (set.missing.iter())
            .filter(|r| tried.insert(r.op_hash))
            .cloned()
            .collect();
        let before = fetched.len();
        for op_ref in wanted {
            // A dep we cannot fetch now (not yet on the replicas we reach,
            // or the read failed) leaves the op unjudged: answered retry
            // later below, never a refusal.
            match op_log::fetch_held(deps, &op_ref, &logs).await {
                Ok(Some(dep)) => {
                    held.push(dep.clone());
                    fetched.push(dep);
                }
                Ok(None) => {}
                Err(e) => tracing::debug!(
                    community = anchor,
                    error = %e,
                    "bootstrap: a dep of the joiner's op is not fetchable now"
                ),
            }
        }
        if fetched.len() > before {
            continue;
        }
        let unheld = unheld_ancestry(op, &held);
        if !unheld.is_empty() {
            return Err(GovernanceRuntimeError::JoinUnresolved {
                missing: unheld.len(),
            });
        }
        let outcome = rekindle_governance::merge::merge_outcome(&cert, &set.ops);
        let state = &outcome.state;
        let admitted = outcome.applied.contains(&op.hash)
            && (holds_joined(state, &op.author) || state.pending_members.contains_key(&op.author));
        return if admitted {
            Ok(fetched)
        } else {
            let reason = (outcome.join_refusals.get(&op.hash).copied())
                .unwrap_or(rekindle_types::join_refusal::JoinRefusal::NotAdmitted);
            Err(GovernanceRuntimeError::JoinNotAdmitted(reason))
        };
    }
}

/// The hashes `op` waits on, directly or through held ops, that we do not
/// hold (`held` includes `op`): its deps and its `prev` (a re-join's own
/// chain), as `verify_cache::waits_on` reads them. A dep we hold but refused
/// (quarantined) is held, so the merge judges the op without it.
fn unheld_ancestry(
    op: &HeldOp,
    held: &[HeldOp],
) -> std::collections::BTreeSet<rekindle_types::governance::OpHash> {
    let by_hash: std::collections::HashMap<_, _> = held.iter().map(|h| (h.hash, h)).collect();
    let mut seen = std::collections::BTreeSet::from([op.hash]);
    let mut queue = vec![op];
    let mut unheld = std::collections::BTreeSet::new();
    while let Some(next) = queue.pop() {
        let Ok(decoded) = rekindle_codec::governance_op::decode_op(&next.encoded) else {
            continue;
        };
        let prev = decoded.op.prev.filter(|_| decoded.op.seq > 0);
        let waits = (decoded.op.deps.iter().map(|d| d.op_hash)).chain(prev);
        for hash in waits {
            if !seen.insert(hash) {
                continue;
            }
            match by_hash.get(&hash) {
                Some(dep) => queue.push(dep),
                None => {
                    unheld.insert(hash);
                }
            }
        }
    }
    unheld
}

/// When the join we just admitted (`joined`) takes the last use of an
/// invite we created, revoke that invite citing it (plan SU1): every
/// reader then refuses a join outside the revoke's causal past
/// (`merge::gates::revoke_excluding`), so a join citing stale deps and
/// ground to a lower hash can never displace the one we admitted. The
/// revoke is queued (we answer within the `app_call` window); our frontier
/// holds the join, so the revoke cites it. An invite already revoked has
/// left `state.invites`, so a repeated request revokes nothing.
///
/// # Errors
/// The revoke could not be authored or stored.
async fn revoke_if_used_up<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    joined: &HeldOp,
    state: &rekindle_governance::state::GovernanceState,
) -> Result<(), GovernanceRuntimeError> {
    use rekindle_types::governance::GovernanceEntry;
    let Ok(decoded) = rekindle_codec::governance_op::decode_op(&joined.encoded) else {
        return Ok(());
    };
    let GovernanceEntry::MemberJoined {
        invite_proof: Some(proof),
        ..
    } = decoded.op.body
    else {
        return Ok(());
    };
    let me = deps
        .community_membership(anchor)
        .and_then(|m| m.my_pseudonym_hex);
    let used_up = (state.invites.iter())
        .filter(|(_, i)| i.code_hash == proof.code_hash)
        .min_by_key(|(id, _)| **id)
        .filter(|(_, i)| i.uses >= i.max_uses)
        .filter(|(_, i)| me.as_deref() == Some(hex::encode(i.creator_pseudonym.0).as_str()))
        .map(|(id, _)| *id);
    if let Some(invite_id) = used_up {
        crate::apply::write_entry_queued(
            deps,
            anchor,
            GovernanceEntry::InviteRevoked { invite_id },
        )
        .await?;
        tracing::info!(
            community = anchor,
            "a used-up invite revoked at its last admission"
        );
    }
    Ok(())
}

/// Whether the merged `state` holds `author`'s `MemberJoined` as admitted:
/// a current member, or one a later `MemberRemoved` removed (a kick is an
/// op after the join, not a verdict on it: ADR 0011, D15). The kicked
/// joiner learns its removal from the frontier walk, as every member learns
/// an op; a refusal is kept for a `MemberJoined` no merge admits (an
/// expired or forged invite, a banned author), which the joiner withdraws
/// (`withdraw_refused_join`). Refusing a kicked joiner withdrew a join the
/// removal had already ended, and that leave raced its re-join.
fn holds_joined(
    state: &rekindle_governance::state::GovernanceState,
    author: &rekindle_types::id::PseudonymKey,
) -> bool {
    !state.bans.contains(author)
        && (state.members.contains_key(author) || state.removed.contains_key(author))
}

/// Whether `op` is the joiner's `MemberJoined`: its op #0, or a re-join,
/// a later `MemberJoined` of the same pseudonym by an author `state` does
/// not hold as a member (it left or was removed; the merge's re-join rule,
/// `gates::rejoined`, decides whether it stands). A member's community
/// pseudonym is fixed per identity and community (`derive_community_pseudonym`),
/// so a re-join continues its own op chain. A re-join we already merged
/// (its `MemberJoined` reached us by gossip or the watch ring before its
/// request, walk M87) holds the author as a member admitted by this very
/// op (`MemberState::joined`), and is answered like any other: the request
/// is idempotent, as the merge is.
fn is_joiners_member_joined(
    op: &op_log::HeldOp,
    op_ref: &OpRef,
    state: &rekindle_governance::state::GovernanceState,
) -> bool {
    let Ok(decoded) = rekindle_codec::governance_op::decode_op(&op.encoded) else {
        return false;
    };
    let joined = matches!(
        decoded.op.body,
        rekindle_types::governance::GovernanceEntry::MemberJoined { .. }
    );
    joined
        && op.author == op_ref.author
        && op.seq == op_ref.author_seq
        && (op.seq == 0 || (state.members.get(&op.author)).is_none_or(|m| m.joined.hash == op.hash))
}

/// Our `bootstrapRefused` answer for `anchor`: the merge does not admit
/// the joiner's `MemberJoined`, for `reason` (plan E3.2 join step 5).
///
/// # Errors
/// The encode failed.
pub fn encode_refusal(
    anchor: &str,
    reason: rekindle_types::join_refusal::JoinRefusal,
) -> Result<Vec<u8>, GovernanceRuntimeError> {
    encode_response(ControlPayload::BootstrapRefused {
        anchor: anchor.to_owned(),
        reason,
    })
}

fn encode_response(payload: ControlPayload) -> Result<Vec<u8>, GovernanceRuntimeError> {
    let bytes = rekindle_codec::capnp_envelope::encode_community_envelope(
        &CommunityEnvelope::control(payload),
    )
    .map_err(|e| GovernanceRuntimeError::Encoding(format!("encode bootstrap response: {e}")))?;
    if bytes.len() > MAX_BOOTSTRAP_RESPONSE_LEN {
        return Err(GovernanceRuntimeError::Encoding(format!(
            "bootstrap response is {} B, over the {MAX_BOOTSTRAP_RESPONSE_LEN} B app_call cap",
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// Ask the inviter at `inviter_route` (its member record `inviter_record`)
/// for its frontier once our `MemberJoined` is durable, or, when it is
/// partial, other members (`crate::bootstrap_peers`), and walk the op DAG
/// from the frontier answered: the ops the walk found (the caller stores
/// the new ones and rebuilds).
///
/// # Errors
/// The inviter's `app_call` failed, the inviter refused
/// ([`GovernanceRuntimeError::BootstrapRefused`]: the reply is neither a
/// `bootstrapResponseV2` nor a `bootstrapPartial` for `anchor`), it was
/// partial and no other member answered
/// ([`GovernanceRuntimeError::BootstrapUnavailable`]), or the walk failed.
pub async fn request_bootstrap<D: GovernanceRuntimeDeps>(
    deps: &D,
    inviter_route: &[u8],
    inviter_record: &str,
    anchor: &str,
    joiner: &BootstrapJoiner,
) -> Result<Vec<HeldOp>, GovernanceRuntimeError> {
    let request = CommunityEnvelope::control(ControlPayload::BootstrapRequest {
        joiner_pseudonym: joiner.pseudonym.clone(),
        anchor: anchor.to_owned(),
        member_joined: joiner.member_joined.clone(),
        member_joined_op: joiner.member_joined_op.clone(),
        joiner_route: joiner.route.clone(),
    });
    let bytes = rekindle_codec::capnp_envelope::encode_community_envelope(&request)
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("encode bootstrap request: {e}")))?;
    let me = joiner.member_joined.author.clone();
    let candidates = crate::bootstrap_peers::candidates(
        &deps.governance_state(anchor).unwrap_or_default(),
        &me,
        inviter_record,
    );
    let port = deps.record_port()?;
    let call =
        |route: Vec<u8>, payload: Vec<u8>| async move { deps.app_call_peer(&route, payload).await };
    let frontier = crate::bootstrap_peers::find_frontier(
        call,
        &*port,
        (anchor, deps.clock().now_ms()),
        inviter_route,
        &candidates,
        &bytes,
    )
    .await?;
    op_log::walk(deps, anchor, &frontier).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::dht_layout::member_record::MAX_FRONTIER_REFS;
    use rekindle_types::governance::OpHash;
    use rekindle_types::id::PseudonymKey;

    /// A Veilid record key at its longest text form.
    fn record_key(i: u8) -> String {
        format!(
            "VLD0:{}:{}",
            "A".repeat(43),
            char::from(b'a' + i % 26).to_string().repeat(43)
        )
    }

    fn op_ref(i: u8) -> OpRef {
        OpRef {
            op_hash: OpHash([i; 32]),
            device_key: [i.wrapping_add(1); 32],
            author: PseudonymKey([i.wrapping_add(1); 32]),
            author_record: record_key(i),
            author_seq: u64::MAX,
        }
    }

    #[test]
    fn the_largest_response_fits_the_app_call_cap() {
        let max_frontier = u8::try_from(MAX_FRONTIER_REFS).unwrap();
        let payload = ControlPayload::BootstrapResponseV2 {
            anchor: record_key(200),
            frontier: (0..max_frontier).map(op_ref).collect(),
        };
        let bytes = encode_response(payload.clone()).expect("fits");
        assert!(
            bytes.len() < MAX_BOOTSTRAP_RESPONSE_LEN,
            "{} B",
            bytes.len()
        );
        let back = rekindle_codec::capnp_envelope::try_decode_community_envelope(&bytes)
            .unwrap()
            .unwrap();
        let Some(ControlPayload::BootstrapResponseV2 { frontier, .. }) = back.into_control() else {
            panic!("not a bootstrapResponseV2");
        };
        assert_eq!(frontier.len(), MAX_FRONTIER_REFS);
    }

    /// A re-join `MemberJoined` (seq 2) by `author`, as held.
    fn rejoin(author: PseudonymKey) -> (HeldOp, OpRef) {
        let op = rekindle_types::governance::GovernanceOp {
            device_key: author.0,
            author: author.clone(),
            seq: 2,
            prev: None,
            deps: Vec::new(),
            issued_at: 1,
            body: rekindle_types::governance::GovernanceEntry::MemberJoined {
                member_record: record_key(7),
                member_record_owner: "o".into(),
                invite_proof: None,
                owner_proof: vec![0; 64],
                first_device: None,
            },
            sig: [0; 64],
        };
        let held = HeldOp {
            hash: OpHash([9; 32]),
            device_key: author.0,
            author: author.clone(),
            author_record: record_key(7),
            seq: 2,
            lamport: 5,
            issued_at: 1,
            encoded: rekindle_codec::governance_op::encode_op(&op).expect("encodes"),
            fate: op_log::OpFate::Applied,
        };
        let op_ref = OpRef {
            op_hash: held.hash,
            device_key: author.0,
            author,
            author_record: record_key(7),
            author_seq: 2,
        };
        (held, op_ref)
    }

    fn member(hash: OpHash) -> rekindle_governance::state::MemberState {
        rekindle_governance::state::MemberState {
            member_record: record_key(7),
            member_record_owner: "o".into(),
            joined: rekindle_governance::op::OpStamp { lamport: 5, hash },
        }
    }

    /// Walk M87: the inviter merged the peer's re-join from gossip before
    /// its `BootstrapRequest` arrived, and refused it as a member's second
    /// join; a re-join admitted by this very op is the joiner's, while one
    /// by a member admitted by another op is not.
    #[test]
    fn a_rejoin_already_merged_is_the_joiners_member_joined() {
        let author = PseudonymKey([4; 32]);
        let (held, op_ref) = rejoin(author.clone());
        let mut state = rekindle_governance::state::GovernanceState::default();
        assert!(is_joiners_member_joined(&held, &op_ref, &state));
        state.members.insert(author.clone(), member(held.hash));
        assert!(is_joiners_member_joined(&held, &op_ref, &state));
        state.members.insert(author, member(OpHash([1; 32])));
        assert!(!is_joiners_member_joined(&held, &op_ref, &state));
    }

    #[test]
    fn a_frontier_over_the_bound_is_refused() {
        let over = u8::try_from(MAX_FRONTIER_REFS + 1).unwrap();
        let payload = ControlPayload::BootstrapResponseV2 {
            anchor: record_key(0),
            frontier: (0..over).map(op_ref).collect(),
        };
        assert!(encode_response(payload).is_err());
    }
}
