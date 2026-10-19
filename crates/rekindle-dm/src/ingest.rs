//! Phase 13 — inbound group DM dispatch (architecture §27.2).
//!
//! A `GroupDmInvite` arrives via `app_message` after the initiator has
//! already created the SMPL record. Our job here is to (1) persist the
//! row in `dms` so the UI can render the invite, (2) emit a
//! `DmEvent::InviteReceived` so the frontend can prompt accept/decline.
//! Acceptance flows through `session::accept_dm_invite`; decline deletes
//! the row. The 1:1 invite (`DmInvite`) went with the 1:1 SMPL path
//! (plan E2.3).

use crate::deps::{DmDeps, DmEvent};
use crate::error::DmError;
use crate::invite::GroupDmParticipant;
use crate::store::DmInvitePending;

/// Unpacked `GroupDmInvite` wire envelope. Groups the distinct fields so
/// the ingest entry point stays under the argument-count budget while
/// each field remains explicit at the dispatcher call site.
pub struct IncomingGroupDmInvite<'a> {
    pub sender_hex: &'a str,
    pub record_key: &'a str,
    pub slot_seed: &'a [u8],
    pub initiator_pseudonym: &'a str,
    pub participants_json: &'a str,
    pub wrapped_mek: &'a [u8],
    pub mek_generation: u32,
}

/// Handle a group DM invite. Architecture §27.2: each `GroupDmParticipant`
/// carries a `public_key` for verification — we find OUR slot by
/// matching against our own identity public key.
pub async fn handle_incoming_group_dm_invite<D: DmDeps + ?Sized>(
    deps: &D,
    invite: IncomingGroupDmInvite<'_>,
) -> Result<(), DmError> {
    let owner_key = deps.owner_key()?;
    let participants: Vec<GroupDmParticipant> = serde_json::from_str(invite.participants_json)
        .map_err(|e| DmError::InvalidInput(format!("invalid participants_json: {e}")))?;

    let secret_bytes = deps.identity_secret()?;
    let identity = rekindle_crypto::Identity::from_secret_bytes(&secret_bytes);
    let my_pubkey_hex = identity.public_key_hex();
    let my_subkey = participants
        .iter()
        .find(|p| p.public_key.eq_ignore_ascii_case(&my_pubkey_hex))
        .ok_or_else(|| {
            DmError::InvalidInput(
                "my pubkey not in group dm participants — invite not addressed to us".into(),
            )
        })?
        .subkey;

    deps.store()
        .persist_invite_pending(
            &owner_key,
            DmInvitePending {
                record_key: invite.record_key.to_string(),
                is_group: true,
                initiator_public_key: invite.sender_hex.to_string(),
                initiator_pseudonym: invite.initiator_pseudonym.to_string(),
                my_subkey,
                participants,
                mek_generation: invite.mek_generation,
                slot_seed_hex: hex::encode(invite.slot_seed),
                wrapped_mek_blob: Some(invite.wrapped_mek.to_vec()),
                created_at: i64::try_from(rekindle_utils::timestamp_ms() / 1000)
                    .unwrap_or(i64::MAX),
            },
        )
        .await?;
    deps.emit_event(DmEvent::InviteReceived {
        record_key: invite.record_key.to_string(),
        sender_pseudonym: invite.initiator_pseudonym.to_string(),
        sender_public_key_hex: invite.sender_hex.to_string(),
    });
    Ok(())
}

/// Handle a `GroupDmLeave` from a peer in a group DM. Treats it as a
/// decline (removes the row). The leave-vs-decline distinction is
/// preserved at the wire-protocol layer; locally both clear the
/// conversation row.
pub async fn handle_incoming_dm_leave<D: DmDeps + ?Sized>(
    deps: &D,
    _sender_hex: &str,
    record_key: &str,
) -> Result<(), DmError> {
    let owner_key = deps.owner_key()?;
    deps.store().decline_invite(&owner_key, record_key).await?;
    deps.dht_release_session(record_key).await;
    Ok(())
}
