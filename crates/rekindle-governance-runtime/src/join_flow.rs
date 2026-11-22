//! The self-sovereign join (plan E3.2, ADR 0011), shared by every host.
//!
//! It works from the DHT with nobody online. The invite names the anchor
//! `A` and the inviter's member record `M`: the joiner pins `A`'s
//! `GenesisCert` to its descriptor owner, reads the inviter's `Frontier`
//! (`M`/3), walks the op DAG by `OpRef` through the authors' op logs, and
//! merges what it found under the cert. The invite is checked against that
//! state (an unknown, revoked or expired invite, or a ban, refuses the
//! join), then the joiner creates its own `M` and signs op #0 of its chain,
//! `MemberJoined{memberRecord, memberRecordOwner, inviteProof}`, and claims
//! its registry slot (E3.4 replaces the claim).
//!
//! **Stops before host state.** Nothing is written to `M` here: the host
//! stores `K_M(p)` and the card's HPKE secret in its vault and the
//! community row first (the keys are durable before anything is
//! published), then stores the walked ops and ours and publishes `M` in
//! §4.4's order (`crate::member_card::publish_own_record`): the card
//! `M`/0, ours on `M`/2 with `M`/3 after it, then the beat `M`/1.

use rekindle_crypto::keypair_text;
use rekindle_governance::op::{invite_proof_bytes, VerifiedOp};
use rekindle_governance::state::GovernanceState;
use rekindle_records::schema;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::governance::{GenesisCert, GovernanceEntry, InviteProof, OpHash};
use rekindle_types::id::PseudonymKey;
use rekindle_types::invite::InviteSecrets;
use rekindle_types::join_refusal::JoinRefusal;
use zeroize::Zeroizing;

use crate::deps::GovernanceRuntimeDeps;
use crate::device_signer;
use crate::error::GovernanceRuntimeError;
use crate::hint_chain::Asker;
use crate::join::{derive_join_identity, InitialPresence, JoinIdentity};
use crate::join_deps::join_deps;
use crate::join_stages::{
    claim_registry_slot, collect_initial_presence_state, ClaimedSlot, SlotClaimCtx,
};
use crate::join_walk::{walk_for_join, WalkedSet};
use crate::op_log::{self, Deps, HeldOp, OpFate};

/// The joiner's own member record, created by the join.
pub struct JoinerRecord {
    pub record_key: String,
    /// `K_M(p)`'s keypair text, for the host's vault.
    pub owner_keypair: Zeroizing<String>,
    /// Our card's X25519 HPKE secret in this community, for the host's
    /// vault (durable before the card is published).
    pub hpke_secret: Zeroizing<[u8; 32]>,
}

/// Everything the join produced. The caller turns this into its own state
/// representation, then stores `walked` and `joined` and publishes `joined`.
pub struct JoinStagesOutcome {
    pub cert_bytes: Vec<u8>,
    pub identity: JoinIdentity,
    pub invite: InviteSecrets,
    pub gov_state: GovernanceState,
    pub member: JoinerRecord,
    /// The ops the walk found (verified by every reader again on merge).
    pub walked: Vec<HeldOp>,
    /// Our op #0, `MemberJoined`.
    pub joined: HeldOp,
    /// The hint the walk resumed from: the caller marks the community
    /// partial from it before storing `walked` (plan E3.2 S7c.8 (1)).
    pub partial: Option<crate::partial::PartialJoin>,
    /// The inviter's route as its `M`/6 held it when we joined, empty when
    /// it held none: where the bootstrap exchange goes.
    pub inviter_route: Vec<u8>,
    pub claimed: ClaimedSlot,
    pub initial_presence: InitialPresence,
}

/// Fetch and decrypt the invite's secrets from their DFLT record.
async fn decode_invite<D: GovernanceRuntimeDeps>(
    deps: &D,
    invite_code: &str,
    secrets_record_key: &str,
) -> Result<InviteSecrets, GovernanceRuntimeError> {
    use base64::Engine as _;
    let encrypted_b64 = crate::invite_secrets::fetch_invite_secrets(deps, secrets_record_key)
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("fetch invite secrets: {e}")))?;
    let encrypted = base64::engine::general_purpose::STANDARD
        .decode(&encrypted_b64)
        .map_err(|e| {
            GovernanceRuntimeError::Encoding(format!("invalid invite secrets encoding: {e}"))
        })?;
    let secrets_json = rekindle_secrets::invite::decrypt_invite_secrets(invite_code, &encrypted)
        .map_err(|e| {
            GovernanceRuntimeError::Crypto(format!("failed to decrypt invite secrets: {e}"))
        })?;
    serde_json::from_slice(&secrets_json)
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("invalid invite secrets: {e}")))
}

fn parse_pseudonym(hex_key: &str) -> Result<PseudonymKey, GovernanceRuntimeError> {
    hex::decode(hex_key)
        .ok()
        .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
        .map(PseudonymKey)
        .ok_or_else(|| GovernanceRuntimeError::Encoding("invite inviter pseudonym".into()))
}

/// The inviter's current route from its `M`/6 (plan E3.2 S7c.8); empty
/// when it holds none or it does not verify, and the join goes on from
/// the DHT alone (no hint, no bootstrap exchange), as with an inviter
/// that is offline.
async fn resolve_inviter_route<D: GovernanceRuntimeDeps>(
    deps: &D,
    anchor: &str,
    invite: &InviteSecrets,
    inviter: &PseudonymKey,
) -> Vec<u8> {
    let record = &invite.inviter_member_record;
    let read = match deps.record_port() {
        Ok(port) => {
            let pin = crate::member_routes::RoutePin::Invite;
            crate::member_routes::read_member_route(&*port, anchor, record, inviter, pin).await
        }
        Err(e) => Err(e.to_string()),
    };
    match read {
        Ok(Some(route)) => route,
        Ok(None) => {
            tracing::info!(community = anchor, "the inviter's record holds no route");
            Vec::new()
        }
        Err(error) => {
            tracing::warn!(community = anchor, %error, "the inviter's route was refused");
            Vec::new()
        }
    }
}

/// Refuse an invite the merged state does not hold as live
/// (`GovernanceState::live_invite`: a join on it would be refused by every
/// reader's `invite_gate`), with the typed reason the inviter's merge would
/// answer (`rekindle_types::join_refusal`): the reasons are derivable from
/// the op DAG this join walked, so naming them discloses nothing new.
///
/// - every use taken by earlier joins: `InviteUsedUp`;
/// - expired, or revoked (an applied `InviteCreated` for `code_hash` that
///   the state no longer holds: `GovernanceState::invites` is
///   `InviteCreated` minus `InviteRevoked`): `InviteNoLongerValid`, one
///   reason for both, as the gate gives, except a revoke whose causal past
///   holds `max_uses` joins (the inviter's revoke at the last use, plan
///   SU1): `InviteUsedUp`;
/// - no applied `InviteCreated` matches: `InviteUnknown`.
///
/// After a resume from a hint, `ops` holds only the post-cut ops: an
/// invite revoked below the cut is not live in the snapshot's state and no
/// post-cut op names it, so it is `InviteUnknown`, as the inviter's
/// resumed gate answers it (`merge::resume_gates`).
pub(crate) fn check_invite(
    state: &GovernanceState,
    walked: (&[VerifiedOp], &[OpHash]),
    code_hash: &str,
    now_secs: u64,
) -> Result<PseudonymKey, GovernanceRuntimeError> {
    if let Some(invite) = state.live_invite(code_hash, now_secs) {
        return Ok(invite.creator_pseudonym.clone());
    }
    let held = (state.invites.iter())
        .filter(|(_, i)| i.code_hash == code_hash)
        .min_by_key(|(id, _)| **id)
        .map(|(_, i)| i);
    let reason = match held {
        Some(invite) if invite.expires_at.is_some_and(|at| at <= now_secs) => {
            JoinRefusal::InviteNoLongerValid
        }
        Some(_) => JoinRefusal::InviteUsedUp,
        None => revoked_reason(walked, code_hash).unwrap_or(JoinRefusal::InviteUnknown),
    };
    Err(GovernanceRuntimeError::InviteRefused(reason))
}

/// Why an invite `code_hash` created by an applied op of the walked set is
/// no longer held (plan SU1): `InviteUsedUp` when an applied
/// `InviteRevoked` of it holds `max_uses` applied joins on it in its causal
/// past (the inviter revokes the invite at the join that fills it,
/// `bootstrap::revoke_if_used_up`), else `InviteNoLongerValid` (revoked or
/// expired, one reason); `None` when no applied op created it.
fn revoked_reason(
    (ops, applied): (&[VerifiedOp], &[OpHash]),
    code_hash: &str,
) -> Option<JoinRefusal> {
    if !created_in((ops, applied), code_hash) {
        return None;
    }
    let is_applied = |op: &&VerifiedOp| applied.contains(&op.hash());
    let used_up = (ops.iter().filter(is_applied)).any(|revoke| {
        let GovernanceEntry::InviteRevoked { invite_id } = revoke.body() else {
            return false;
        };
        let Some(max_uses) = (ops.iter().filter(is_applied)).find_map(|c| match c.body() {
            GovernanceEntry::InviteCreated {
                invite_id: id,
                code_hash: c,
                max_uses,
                ..
            } if id == invite_id && c == code_hash => Some(*max_uses),
            _ => None,
        }) else {
            return false;
        };
        let past = causal_past(ops, revoke);
        let joins = (ops.iter().filter(is_applied))
            .filter(|j| past.contains(&j.hash()))
            .filter(|j| {
                matches!(j.body(), GovernanceEntry::MemberJoined { invite_proof: Some(p), .. }
                    if p.code_hash == code_hash)
            })
            .count();
        u32::try_from(joins).is_ok_and(|joins| joins >= max_uses)
    });
    Some(if used_up {
        JoinRefusal::InviteUsedUp
    } else {
        JoinRefusal::InviteNoLongerValid
    })
}

/// The hashes of `op`'s causal past among `ops` (deps outside `ops`, below
/// a resume's cut, end the walk).
fn causal_past(ops: &[VerifiedOp], op: &VerifiedOp) -> std::collections::BTreeSet<OpHash> {
    let index: std::collections::HashMap<OpHash, &VerifiedOp> =
        ops.iter().map(|o| (o.hash(), o)).collect();
    let mut past = std::collections::BTreeSet::new();
    let mut todo: Vec<OpHash> = op.op().deps.iter().map(|r| r.op_hash).collect();
    while let Some(hash) = todo.pop() {
        if past.insert(hash) {
            if let Some(dep) = index.get(&hash) {
                todo.extend(dep.op().deps.iter().map(|r| r.op_hash));
            }
        }
    }
    past
}

/// Whether an applied op of the walked set created the invite `code_hash`.
fn created_in((ops, applied): (&[VerifiedOp], &[OpHash]), code_hash: &str) -> bool {
    ops.iter().any(|op| {
        matches!(op.body(), GovernanceEntry::InviteCreated { code_hash: c, .. } if c == code_hash)
            && applied.contains(&op.hash())
    })
}

/// Create our member record `M(p)`; its lease is released (the op-log
/// sender reacquires it with `K_M(p)`).
async fn create_member_record<D: GovernanceRuntimeDeps>(
    deps: &D,
) -> Result<(JoinerRecord, String), GovernanceRuntimeError> {
    let info = deps.create_record(&schema::member_record()).await?;
    deps.release_record(info.lease).await;
    let text = Zeroizing::new(info.owner_keypair.clone().ok_or_else(|| {
        GovernanceRuntimeError::Adapter("join: the member record has no owner keypair".into())
    })?);
    let (public, _) = keypair_text::parse_keypair_text(&text)
        .map_err(|e| GovernanceRuntimeError::Crypto(format!("member record owner: {e}")))?;
    let owner = keypair_text::public_key_text(&public);
    Ok((
        JoinerRecord {
            record_key: info.record_key,
            owner_keypair: text,
            hpke_secret: crate::member_card::new_hpke_secret(deps.rng().as_ref()),
        },
        owner,
    ))
}

/// Our op #0: `MemberJoined` with the invite proof (the invite's join key,
/// derived from the code we hold, over `INVITE_PROOF ‖ anchor ‖ our
/// pseudonym ‖ memberRecord ‖ code hash`, plan E3.2 S10) and the owner
/// proof (`K_M` over `MEMBER_RECORD_BINDING ‖ anchor ‖ pseudonym ‖
/// memberRecord`, ADR 0011 "Binding"). Its deps are shallow (plan E3.2
/// S7c.0, [`join_deps`](crate::join_deps::join_deps)): a `Checkpoint` op
/// of `walked`'s hint, the one it resumed from or its full merge's
/// selected one (else the creator's last genesis op), and `inviter`'s
/// latest op (none after a resume when the inviter has no post-cut op),
/// not the whole frontier.
fn joined_op(
    anchor: &str,
    cert: &GenesisCert,
    walked: &WalkedSet,
    keys: (&SigningKey, &SigningKey),
    member: (&JoinerRecord, String),
    invite: (&str, &PseudonymKey),
    now_secs: u64,
) -> Result<HeldOp, GovernanceRuntimeError> {
    let (code, inviter) = invite;
    let (member, owner) = member;
    // `p` endorses this device's first op key `d`, which signs (ADR 0022).
    let (key, device) = keys;
    let record = member.record_key.as_str();
    let joiner = PseudonymKey(key.verifying_key().to_bytes());
    let code_hash = rekindle_secrets::invite::hash_invite_code(code);
    let proof = invite_proof_bytes(anchor, &joiner, record, &code_hash);
    let sig = rekindle_secrets::invite::sign_invite_join(code, &proof).to_vec();
    let (_, owner_secret) = keypair_text::parse_keypair_text(&member.owner_keypair)
        .map_err(|e| GovernanceRuntimeError::Crypto(format!("member record owner: {e}")))?;
    let owner_secret = Zeroizing::new(owner_secret);
    let owner_proof = rekindle_secrets::member_record::sign_member_record_binding(
        &owner_secret,
        anchor,
        &key.verifying_key().to_bytes(),
        record,
    );
    let body = GovernanceEntry::MemberJoined {
        member_record: record.to_owned(),
        member_record_owner: owner.clone(),
        invite_proof: Some(InviteProof { code_hash, sig }),
        owner_proof: owner_proof.to_vec(),
        first_device: Some(device_signer::member_first_endorsement(
            key,
            anchor,
            device,
            (record, &owner),
            &walked.merged.state,
        )),
    };
    let (set, outcome) = (&walked.set, &walked.merged);
    let hint = walked.hint.as_ref();
    let deps = Deps::Join(join_deps(cert, set, outcome, hint, &joiner, inviter)?);
    let state = &outcome.state;
    let signer = device_signer::OpSigner {
        author: joiner,
        device,
    };
    let (op, encoded) = op_log::build_op(anchor, cert, state, set, signer, body, deps, now_secs)?;
    Ok(op_log::held(
        &op,
        record.to_owned(),
        encoded,
        OpFate::Applied,
    ))
}

/// Run the join.
///
/// # Errors
/// The anchor does not pin, the invite does not decode or is not live in
/// the walked state, we are banned, or a DHT step failed.
pub async fn run_join_stages<D: GovernanceRuntimeDeps>(
    deps: &D,
    link: &rekindle_types::invite::InviteLink,
) -> Result<JoinStagesOutcome, GovernanceRuntimeError> {
    let anchor = link.anchor.as_str();
    let (cert, cert_bytes) = crate::genesis::read_pinned(deps, anchor).await?;
    let identity_secret = deps
        .identity_secret()
        .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?;
    let identity = derive_join_identity(&identity_secret, anchor);
    let invite = decode_invite(
        deps,
        link.invite_code.as_str(),
        link.secrets_record_key.as_str(),
    )
    .await?;
    if invite.anchor != anchor {
        return Err(GovernanceRuntimeError::Adapter(
            "the invite names another community".into(),
        ));
    }

    // The op set from the inviter's frontier, merged under the pinned cert:
    // above a hint's cut from its snapshot when the inviter gives one that
    // verifies (plan E3.2 S7c.5), else in full.
    let code_hash = rekindle_secrets::invite::hash_invite_code(link.invite_code.as_str());
    let read = crate::join_frontier::read_frontier(deps, &invite.inviter_member_record).await?;
    let inviter_pseudonym = parse_pseudonym(&invite.inviter_pseudonym)?;
    tracing::info!(
        community = anchor,
        frontier_seq = read.seq,
        minimum = link.frontier_seq,
        heads = read.frontier.heads.len(),
        frontier = %read.short_hash,
        "join: the inviter's frontier read"
    );
    let seed =
        crate::join_frontier::invite_op_seed(deps, link, &invite.inviter_member_record, &read)
            .await?;
    let frontier = read.frontier;
    let inviter_route = resolve_inviter_route(deps, anchor, &invite, &inviter_pseudonym).await;
    let asker = Asker {
        me: &identity.pseudonym_signing,
        route: &inviter_route,
        code: link.invite_code.as_str(),
    };
    let walked = walk_for_join(deps, anchor, &cert, (&frontier, seed), &asker).await?;
    tracing::info!(
        community = anchor,
        walked = walked.held.len(),
        "join: the op set walked"
    );
    let gov_state = walked.merged.state.clone();

    if gov_state.bans.contains(&identity.pseudo) {
        return Err(GovernanceRuntimeError::Adapter(
            "You are banned from this community".into(),
        ));
    }
    let now = deps.clock().now_secs();
    let applied = walked.merged.applied.as_slice();
    let inviter = check_invite(&gov_state, (&walked.set.ops, applied), &code_hash, now)
        .inspect_err(|refused| {
            tracing::info!(community = anchor, source = "check_invite", %refused, "join refused");
        })?;
    if inviter != inviter_pseudonym {
        return Err(GovernanceRuntimeError::Adapter(
            "the invite's secrets name another inviter than its InviteCreated".into(),
        ));
    }

    let (member, member_owner) = create_member_record(deps).await?;
    // This device's first op key here, minted and dropped in this block:
    // it never lives across an await of the join.
    let joined = {
        let device = device_signer::device_op_key(deps, anchor, true)?;
        joined_op(
            anchor,
            &cert,
            &walked,
            (&identity.pseudonym_signing, &device),
            (&member, member_owner),
            (link.invite_code.as_str(), &inviter),
            now,
        )?
    };

    // The registry slot, as before E3.4. A claim is not an access-control
    // point (`o_cnt: 0`): readers count only members whose `MemberJoined`
    // they verified.
    let claimed = claim_registry_slot(
        deps,
        &invite.slot_seed,
        SlotClaimCtx {
            community_id: anchor,
            invite_registry_key: &invite.registry_key,
            inviter_pseudonym: Some(&inviter),
            my_pseudo: &identity.pseudo,
            pseudonym_signing: &identity.pseudonym_signing,
            gov_state: &gov_state,
            join_status_label: deps.identity_status().as_wire_str(),
            // No name (plan E3.4 S6), as origin's.
            display_name: None,
        },
    )
    .await?;
    let initial_presence = collect_initial_presence_state(
        deps,
        &claimed.registry_key,
        claimed.local_subkey,
        &identity.pseudo_hex,
        &claimed.occupied_subkeys,
    )
    .await;

    Ok(JoinStagesOutcome {
        cert_bytes,
        identity,
        invite,
        gov_state,
        member,
        walked: walked.held,
        joined,
        partial: walked.partial,
        inviter_route,
        claimed,
        initial_presence,
    })
}

#[cfg(test)]
#[path = "join_flow_invite_tests.rs"]
mod invite_tests;

#[cfg(test)]
mod tests {
    use base64::Engine as _;

    /// Guards the choice of alphabet/padding, not base64 itself: the
    /// invite blob is written with STANDARD, and a mismatch here would
    /// surface as a confusing decryption failure rather than a decode
    /// error.
    #[test]
    fn invite_blob_uses_standard_base64() {
        let enc = base64::engine::general_purpose::STANDARD;
        assert_eq!(enc.decode("Zm9vYmFy").unwrap(), b"foobar");
        assert_eq!(enc.decode("Zg==").unwrap(), b"f");
        assert!(enc.decode("Zm9v!!").is_err());
    }
}
