//! Minting an invite, once, for both tracks.
//!
//! An invite is not a code — it is a code plus an encrypted blob
//! carrying everything a joiner cannot derive: the registry key, the
//! **shared slot seed**, the current MEK, and the channel record keys.
//! The blob lives in its own DFLT record and governance carries only a
//! pointer, because it is multi-KB and would overflow the per-subkey
//! SMPL cap if inlined.
//!
//! ## What the daemon was doing instead
//!
//! `transport::operations::invites::create_invite` appended an
//! `InviteEntry` to the **v1.0 governance-manifest invites subkey** with
//! `encrypted_secrets: None`, and wrote no `InviteCreated` entry. Two
//! consequences, both silent:
//!
//! * The invite could not be used at all. `join_flow::decode_invite`
//!   needs a secrets pointer to reach the slot seed — the registry's 255
//!   member keys come from the creator's seed, so a joiner that cannot
//!   read it can never produce a matching writer. It failed with
//!   "invite has no secrets pointer".
//! * With no `InviteCreated` in the CRDT there was nothing for
//!   `inspect_invite_in_entries` to find, so revocation, expiry and the
//!   per-inviter `invite_quota` cap never applied to it.
//!
//! The quota is the slot-exhaustion control
//! (`rekindle_governance::invite_quota`), which is why the second point
//! matters as much as the first.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::invite::InviteSecrets;

use crate::apply;
use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::invite_secrets::publish_invite_secrets;

/// A freshly minted invite.
#[derive(Debug, Clone)]
pub struct MintedInvite {
    /// The raw code. Shown to the user once; only its hash is published.
    pub code: String,
    pub code_hash: String,
    /// Where the encrypted secrets landed — the joiner needs this and
    /// the code together.
    pub secrets_record_key: String,
    pub invite_id: [u8; 16],
    pub expires_at: Option<u64>,
    /// Our `InviteCreated` by hash and seq in our chain, and the seq of our
    /// frontier (`M`/3) once it landed: the link carries them, so a joiner
    /// whose frontier read is older fetches the op by ref (plan IV1b, lead
    /// decision (28)).
    pub invite_op: rekindle_types::governance::OpHash,
    pub invite_op_seq: u64,
    pub frontier_seq: u32,
}

/// Mint an invite and announce it in governance.
pub async fn create_invite<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    max_uses: u32,
    expires_in_seconds: Option<u64>,
) -> Result<MintedInvite, GovernanceRuntimeError> {
    // Readers refuse a `max_uses` outside `1..=MAX_USES_PER_INVITE` (plan
    // E3.2 (d)), so it is refused here, before any secret is published.
    if !rekindle_governance::invite_quota::check_max_uses_cap(max_uses) {
        return Err(GovernanceRuntimeError::OutOfBounds { field: "max_uses" });
    }
    // An invite is never held (plan E3.2 S7d-keys.8): while the unlock
    // barrier catches up it is refused at once, before any network step;
    // every later step is bounded by the command's network deadline.
    crate::unlock_hold::refuse_unheld(&deps.unlock_barrier(community_id), community_id)?;
    let invite_id = rekindle_utils::random::id_bytes_16_from(deps.rng().as_ref());
    let landing = crate::deps::OpLanding::invite_landed(invite_id);
    let membership = deps
        .community_membership(community_id)
        .ok_or_else(|| GovernanceRuntimeError::CommunityNotFound(community_id.to_string()))?;
    let gov_state = deps
        .governance_state(community_id)
        .ok_or_else(|| GovernanceRuntimeError::GovernanceStateMissing(community_id.to_string()))?;

    let registry_key = membership.member_registry_key.clone().ok_or_else(|| {
        GovernanceRuntimeError::Adapter("no registry key for community".to_string())
    })?;
    let slot_seed = membership
        .slot_seed_hex
        .clone()
        .ok_or_else(|| GovernanceRuntimeError::SlotSeedMissing(community_id.to_string()))?;

    // Architecture §6.2 — an invite must bootstrap the joiner into the
    // PRIMARY (segment-0) registry: the joiner treats the invite's
    // registry as `slot_range_start 0` and derives its slot keypair from
    // that. Segment 0 is implicit and never listed in
    // `gov_state.segments`, so a match against a segment-≥1 registry
    // means the value is corrupt and the invite would mis-route every
    // joiner who used it. Refuse rather than ship the break.
    if let Some(segment) = gov_state
        .segments
        .iter()
        .find(|s| s.segment_index != 0 && s.registry_key == registry_key)
    {
        return Err(GovernanceRuntimeError::Adapter(format!(
            "invite registry key is the segment-{} registry, not the primary",
            segment.segment_index
        )));
    }

    // The joiner's walk of the op DAG starts at our frontier (`M`/3), on
    // our op record (below).
    let me = deps
        .identity_secret()
        .map(|secret| {
            let key = rekindle_secrets::derive::derive_community_pseudonym(&secret, community_id);
            rekindle_types::id::PseudonymKey(key.verifying_key().to_bytes())
        })
        .ok_or(GovernanceRuntimeError::IdentitySecretUnavailable)?;
    // The record our ops and `Frontier` live on, where a joiner reads our
    // frontier and fetches our `InviteCreated` by ref: this device's own
    // (ADR 0022 rule 3, subkeys 2 and 3 are "that device's own op chain and
    // view"), a further device's `M(p,d)`.
    let inviter_member_record =
        crate::device_signer::own_op_record(deps, community_id, &gov_state)?;

    let code = hex::encode(rekindle_utils::random::id_bytes_16_from(
        deps.rng().as_ref(),
    ));
    let code_hash = rekindle_secrets::invite::hash_invite_code(&code);

    let secrets = InviteSecrets {
        anchor: community_id.to_string(),
        inviter_member_record,
        registry_key,
        inviter_pseudonym: hex::encode(me.0),
        slot_seed,
        // From merged governance, not the local membership record: the
        // name a joiner is shown before joining should be the one the
        // community agrees on, not whatever this peer cached.
        community_name: gov_state
            .metadata
            .as_ref()
            .map(|m| m.name.clone())
            .unwrap_or_default(),
    };
    let secrets_json = serde_json::to_vec(&secrets)
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("serialize invite secrets: {e}")))?;
    let encrypted = rekindle_secrets::invite::encrypt_invite_secrets(&code, &secrets_json)
        .map_err(|e| GovernanceRuntimeError::Crypto(format!("encrypt invite secrets: {e}")))?;
    let encrypted_b64 = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(&encrypted)
    };

    let secrets_record_key = tokio::time::timeout(
        landing.remaining(),
        publish_invite_secrets(deps, &encrypted_b64),
    )
    .await
    .map_err(|_| apply::invite_timed_out())??;

    let expires_at = expires_in_seconds.map(|s| deps.clock().now_secs() + s);
    // The link is handed out only once its op and our frontier are on `M`
    // (plan E3.2 S6 (4)): a joiner walks what is on the DHT.
    let created = apply::write_invite_created(
        deps,
        community_id,
        GovernanceEntry::InviteCreated {
            invite_id,
            code_hash: code_hash.clone(),
            max_uses,
            expires_at,
            secrets_record_key: secrets_record_key.clone(),
        },
        landing,
    )
    .await;
    if let Err(GovernanceRuntimeError::NotYetPublished) = created {
        // Refused, but the op is queued and lands later: its tombstone goes
        // on the same stream behind it now, so the invite is never listed or
        // usable and its quota slot frees once both land (S6b). Should this
        // write fail, the revoke row queued with the op revokes it.
        apply::write_entry_queued(
            deps,
            community_id,
            GovernanceEntry::InviteRevoked { invite_id },
        )
        .await?;
    }
    created?;
    let (invite_op, invite_op_seq) = own_invite_op(deps, community_id, &me, invite_id).await?;
    let frontier_seq = landed_frontier_seq(deps, &secrets.inviter_member_record).await?;

    Ok(MintedInvite {
        code,
        code_hash,
        secrets_record_key,
        invite_id,
        expires_at,
        invite_op,
        invite_op_seq,
        frontier_seq,
    })
}

/// Our `InviteCreated` of `invite_id` in our op store: its hash and seq.
async fn own_invite_op<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    me: &rekindle_types::id::PseudonymKey,
    invite_id: [u8; 16],
) -> Result<(rekindle_types::governance::OpHash, u64), GovernanceRuntimeError> {
    let held = deps.op_store().governance_ops(community_id).await?;
    let set = crate::op_log::verify_held_with(deps, community_id, &held);
    (set.ops.iter())
        .filter(|op| op.author() == me)
        .find(|op| {
            matches!(op.body(), GovernanceEntry::InviteCreated { invite_id: id, .. } if *id == invite_id)
        })
        .map(|op| (op.hash(), op.op().seq))
        .ok_or_else(|| GovernanceRuntimeError::Adapter("our InviteCreated is not stored".into()))
}

/// The seq our frontier (`M`/3) holds locally once the invite's op and
/// frontier landed (`apply::write_invite_created`): the minimum a joiner's
/// read of it must meet.
async fn landed_frontier_seq<D: GovernanceRuntimeDeps>(
    deps: &D,
    record: &str,
) -> Result<u32, GovernanceRuntimeError> {
    use rekindle_types::dht_layout::member_record::FRONTIER;
    let lease = deps
        .acquire_record(record, None, rekindle_records::port::OpenIntent::Check)
        .await?;
    let seqs = deps.inspect_dht_record_local_seqs(lease).await;
    deps.release_record(lease).await;
    let seq = seqs?.get(FRONTIER as usize).copied().flatten();
    seq.and_then(|s| u32::try_from(s).ok())
        .ok_or_else(|| GovernanceRuntimeError::Adapter("our frontier holds no seq".into()))
}

/// Revoke an invite by writing the tombstone every peer merges.
///
/// The v1.0 path rewrote the manifest invites subkey — a community-wide
/// entry `o_cnt: 0` credentials nobody for, and one no reader consults.
pub async fn revoke_invite<D: GovernanceRuntimeDeps>(
    deps: &D,
    community_id: &str,
    invite_id: [u8; 16],
) -> Result<(), GovernanceRuntimeError> {
    apply::write_entry(
        deps,
        community_id,
        GovernanceEntry::InviteRevoked { invite_id },
    )
    .await?;
    Ok(())
}
