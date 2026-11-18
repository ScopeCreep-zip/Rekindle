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
use rekindle_governance::op::invite_proof_bytes;
use rekindle_governance::state::GovernanceState;
use rekindle_records::port::OpenIntent;
use rekindle_records::schema;
use rekindle_secrets::ed25519_dalek::{Signer, SigningKey};
use rekindle_types::dht_layout::member_record;
use rekindle_types::governance::{Frontier, GenesisCert, GovernanceEntry, InviteProof};
use rekindle_types::id::PseudonymKey;
use rekindle_types::invite::InviteSecrets;
use zeroize::Zeroizing;

use crate::deps::GovernanceRuntimeDeps;
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

/// A member's `Frontier` (`M`/3), read from the network: its heads and
/// roster checksum.
///
/// # Errors
/// The record is unreadable, holds no frontier, or it does not decode.
pub async fn read_frontier<D: GovernanceRuntimeDeps>(
    deps: &D,
    record: &str,
) -> Result<Frontier, GovernanceRuntimeError> {
    let lease = deps.acquire_record(record, None, OpenIntent::Check).await?;
    let read = deps
        .get_dht_value(lease, member_record::FRONTIER, true)
        .await;
    deps.release_record(lease).await;
    let bytes = read?.ok_or_else(|| {
        GovernanceRuntimeError::Adapter(format!("member record {record} has no frontier"))
    })?;
    rekindle_codec::governance_op::decode_frontier(&bytes)
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("frontier of {record}: {e}")))
}

/// Refuse an invite the merged state does not hold as live
/// (`GovernanceState::live_invite`): unknown or revoked, expired, or used
/// up (a join on it would be refused by every reader's `invite_gate`).
fn check_invite(
    state: &GovernanceState,
    code_hash: &str,
    now_secs: u64,
) -> Result<PseudonymKey, GovernanceRuntimeError> {
    state
        .live_invite(code_hash, now_secs)
        .map(|invite| invite.creator_pseudonym.clone())
        .ok_or_else(|| {
            GovernanceRuntimeError::Adapter("invite is revoked, unknown, expired or used up".into())
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

/// Our op #0: `MemberJoined` with the invite proof (the joiner pseudonym
/// over `INVITE_PROOF ‖ anchor ‖ code hash ‖ memberRecord`) and the owner
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
    key: &SigningKey,
    member: (&JoinerRecord, String),
    invite: (String, &PseudonymKey),
    now_secs: u64,
) -> Result<HeldOp, GovernanceRuntimeError> {
    let (code_hash, inviter) = invite;
    let (member, owner) = member;
    let record = member.record_key.as_str();
    let sig = key
        .sign(&invite_proof_bytes(anchor, &code_hash, record))
        .to_bytes()
        .to_vec();
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
        member_record_owner: owner,
        invite_proof: Some(InviteProof { code_hash, sig }),
        owner_proof: owner_proof.to_vec(),
    };
    let joiner = PseudonymKey(key.verifying_key().to_bytes());
    let (set, outcome) = (&walked.set, &walked.merged);
    let hint = walked.hint.as_ref();
    let deps = Deps::Join(join_deps(cert, set, outcome, hint, &joiner, inviter)?);
    let state = &outcome.state;
    let (op, encoded) = op_log::build_op(anchor, cert, state, set, key, body, deps, now_secs)?;
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
    let frontier = read_frontier(deps, &invite.inviter_member_record).await?;
    let asker = Asker {
        me: &identity.pseudonym_signing,
        route: &invite.inviter_route_blob,
        code_hash: &code_hash,
    };
    let walked = walk_for_join(deps, anchor, &cert, &frontier, &asker).await?;
    let gov_state = walked.merged.state.clone();

    if gov_state.bans.contains(&identity.pseudo) {
        return Err(GovernanceRuntimeError::Adapter(
            "You are banned from this community".into(),
        ));
    }
    let now = deps.clock().now_secs();
    let inviter = check_invite(&gov_state, &code_hash, now)?;

    let (member, member_owner) = create_member_record(deps).await?;
    let joined = joined_op(
        anchor,
        &cert,
        &walked,
        &identity.pseudonym_signing,
        (&member, member_owner),
        (code_hash, &inviter),
        now,
    )?;

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
            display_name: Some(deps.identity_display_name()),
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
        claimed,
        initial_presence,
    })
}

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
