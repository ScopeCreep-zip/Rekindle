//! The self-sovereign join, stages 1-5, shared by every host.
//!
//! Architecture §6.2 steps 1-9: governance snapshot → identity → ban
//! check → invite decode → slot claim → initial presence. Generic over
//! `GovernanceRuntimeDeps`, so the Tauri host and the daemon run the
//! same sequence rather than one each.
//!
//! That mattered: the daemon had no v2.0 join at all. It submitted a
//! request to an inbox and waited for an operator to *assign* it a slot,
//! then derived its slot seed from its own identity key —
//! `derive_slot_seed(signing_key, governance_key, slot)`. The registry's
//! 255 member keys come from the **creator's** shared seed
//! (`create_segment`), so a locally-derived seed can never produce a
//! matching writer and veilid rejects the presence write. The seed has
//! to come from the invite, which is what this does.
//!
//! **Stops before host state.** Everything after step 5 — building the
//! host's community record, persisting it, spawning loops — differs per
//! host and stays there. This is the Schwarzschild boundary: shared
//! orchestration up to the point where host state begins.
//!
//! The BootstrapBundle optimisation is deliberately *not* here. The
//! architecture calls it "convenience, not trust" and says the join is
//! self-sovereign regardless of inviter availability, so a host may
//! fetch it as an extra step; skipping it costs latency, never
//! correctness.

use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;

use crate::deps::GovernanceRuntimeDeps;
use crate::error::GovernanceRuntimeError;
use crate::join::{
    derive_join_identity, inspect_invite_in_entries, InitialPresence, InviteGovStatus, JoinIdentity,
};
use crate::join_stages::{
    claim_registry_slot, collect_initial_presence_state, ClaimedSlot, GovernanceSnapshot,
    SlotClaimCtx,
};

/// What the invite yielded, once decrypted and checked against
/// governance.
pub struct InviteContext {
    pub registry_key: String,
    pub slot_seed_hex: String,
    pub mek_wire_bytes: String,
    pub community_name: String,
    pub inviter_pseudonym: Option<PseudonymKey>,
    pub inviter_route_blob: Vec<u8>,
}

/// Everything stages 1-5 produced. The caller turns this into its own
/// state representation.
pub struct JoinStagesOutcome {
    pub snapshot: GovernanceSnapshot,
    pub identity: JoinIdentity,
    pub invite: InviteContext,
    pub claimed: ClaimedSlot,
    pub initial_presence: InitialPresence,
}

/// Decrypt the invite and reconcile it with governance.
///
/// Governance is consulted to *enforce* revocation and expiry when the
/// entry is visible, and to recover the inviter for the invite-quota
/// check — never as the sole source of the secrets pointer, which rides
/// in the deep link (chiral §12).
async fn decode_invite<D: GovernanceRuntimeDeps>(
    deps: &D,
    invite_code: &str,
    link_secrets_record_key: Option<&str>,
    all_entries: &[(PseudonymKey, Vec<GovernanceEntry>)],
) -> Result<InviteContext, GovernanceRuntimeError> {
    let code_hash = rekindle_secrets::invite::hash_invite_code(invite_code);
    let status = inspect_invite_in_entries(all_entries, &code_hash);
    match status {
        InviteGovStatus::Revoked => {
            return Err(GovernanceRuntimeError::Adapter(
                "invite has been revoked".into(),
            ))
        }
        InviteGovStatus::Expired => {
            return Err(GovernanceRuntimeError::Adapter("invite has expired".into()))
        }
        _ => {}
    }

    let secrets_record_key = link_secrets_record_key
        .map(str::to_string)
        .or_else(|| match &status {
            InviteGovStatus::Active {
                secrets_record_key, ..
            } => Some(secrets_record_key.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            GovernanceRuntimeError::Adapter(
                "invite has no secrets pointer (no link pointer and no governance entry)".into(),
            )
        })?;
    let inviter_pseudonym = match status {
        InviteGovStatus::Active { inviter, .. } => Some(inviter),
        _ => None,
    };

    // Governance carries only a pointer; the encrypted blob lives in its
    // own DFLT record.
    let encrypted_b64 = crate::invite_secrets::fetch_invite_secrets(deps, &secrets_record_key)
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("fetch invite secrets: {e}")))?;
    let encrypted = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(&encrypted_b64)
            .map_err(|e| {
                GovernanceRuntimeError::Encoding(format!("invalid invite secrets encoding: {e}"))
            })?
    };
    let secrets_json = rekindle_secrets::invite::decrypt_invite_secrets(invite_code, &encrypted)
        .map_err(|e| {
            GovernanceRuntimeError::Crypto(format!("failed to decrypt invite secrets: {e}"))
        })?;
    let secrets: rekindle_types::invite::InviteSecrets = serde_json::from_slice(&secrets_json)
        .map_err(|e| GovernanceRuntimeError::Encoding(format!("invalid invite secrets: {e}")))?;

    Ok(InviteContext {
        registry_key: secrets.registry_key,
        slot_seed_hex: secrets.slot_seed,
        mek_wire_bytes: secrets.mek_wire_bytes,
        community_name: secrets.community_name,
        inviter_pseudonym,
        inviter_route_blob: secrets.inviter_route_blob,
    })
}

/// Run join stages 1-5.
///
/// The joiner does every step itself — no operator assigns anything.
/// `claim_registry_slot` is the compare-and-swap that makes that safe
/// under concurrent joins.
pub async fn run_join_stages<D: GovernanceRuntimeDeps>(
    deps: &D,
    governance_key: &str,
    invite_code: &str,
    secrets_record_key: Option<&str>,
) -> Result<JoinStagesOutcome, GovernanceRuntimeError> {
    // 1. Multi-segment governance snapshot (DHT scan + signature verify
    //    + CRDT re-merge).
    let snapshot = crate::join_stages::load_governance_snapshot(deps, governance_key).await?;

    // 2. Our pseudonym for this community — HKDF(master_secret,
    //    community_id), unlinkable across communities.
    let identity_secret = deps.identity_secret().ok_or_else(|| {
        GovernanceRuntimeError::Adapter("identity secret not available (locked?)".into())
    })?;
    let identity = derive_join_identity(&identity_secret, governance_key);

    // 3. Ban check, client-side. A banned pseudonym aborts here rather
    //    than burning a registry slot it would be dropped from anyway.
    if snapshot.gov_state.bans.contains(&identity.pseudo) {
        return Err(GovernanceRuntimeError::Adapter(
            "You are banned from this community".into(),
        ));
    }

    // 4. Decode the invite — this is where the shared slot seed comes
    //    from. Deriving it locally cannot work: the registry's member
    //    keys were generated from the creator's seed.
    let invite =
        decode_invite(deps, invite_code, secrets_record_key, &snapshot.all_entries).await?;

    // 5. Claim a slot (CAS, retrying past contention, auto-expanding
    //    the Plate Gate when every segment is full).
    let display_name = Some(deps.identity_display_name());
    let claimed = claim_registry_slot(
        deps,
        &invite.slot_seed_hex,
        SlotClaimCtx {
            community_id: governance_key,
            invite_registry_key: &invite.registry_key,
            inviter_pseudonym: invite.inviter_pseudonym.as_ref(),
            my_pseudo: &identity.pseudo,
            pseudonym_signing: &identity.pseudonym_signing,
            gov_state: &snapshot.gov_state,
            join_status_label: deps.identity_status().as_wire_str(),
            display_name,
        },
    )
    .await?;

    // 6. Seed the peer set from the slots the claim already inspected —
    //    no second sweep of the registry.
    let initial_presence = collect_initial_presence_state(
        deps,
        &claimed.registry_key,
        claimed.local_subkey,
        &identity.pseudo_hex,
        &claimed.occupied_subkeys,
    )
    .await;

    Ok(JoinStagesOutcome {
        snapshot,
        identity,
        invite,
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
