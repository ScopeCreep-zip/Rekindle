//! Phase 13 — inbound DM dispatch (architecture §27.1).
//!
//! A `DmInvite` / `GroupDmInvite` arrives via `app_message`/`app_call`
//! after the initiator has already created the SMPL record. Our job
//! here is to (1) persist the row in `dms` so the UI can render the
//! invite, (2) emit a `DmEvent::InviteReceived` so the frontend can
//! prompt accept/decline. Acceptance flows through `session::accept_dm_invite`
//! (which opens the record writable); decline sends `DmDecline` and
//! deletes the row.

use crate::deps::{DmDeps, DmEvent};
use crate::error::DmError;
use crate::invite::GroupDmParticipant;
use crate::store::DmInvitePending;

/// Unpacked `DmInvite` wire envelope. Groups the six distinct fields so
/// the ingest entry point stays under the argument-count budget while
/// each field remains explicit at the dispatcher call site.
pub struct IncomingDmInvite<'a> {
    pub sender_hex: &'a str,
    pub record_key: &'a str,
    pub slot_seed: &'a [u8],
    pub alice_pseudonym: &'a str,
    pub alice_subkey: u32,
    pub bob_subkey: u32,
}

/// Handle a 1:1 DM invite from `invite.sender_hex` for the SMPL record
/// allocated at `invite.record_key`. Persists locally; emits an invite event.
pub async fn handle_incoming_dm_invite<D: DmDeps + ?Sized>(
    deps: &D,
    invite: IncomingDmInvite<'_>,
) -> Result<(), DmError> {
    let owner_key = deps.owner_key()?;
    let participants = vec![
        GroupDmParticipant {
            pseudonym: invite.alice_pseudonym.to_string(),
            subkey: invite.alice_subkey,
            public_key: invite.sender_hex.to_string(),
        },
        // Our slot — public_key filled in when we accept and derive.
        GroupDmParticipant {
            pseudonym: String::new(),
            subkey: invite.bob_subkey,
            public_key: String::new(),
        },
    ];
    deps.store()
        .persist_invite_pending(
            &owner_key,
            DmInvitePending {
                record_key: invite.record_key.to_string(),
                is_group: false,
                initiator_public_key: invite.sender_hex.to_string(),
                initiator_pseudonym: invite.alice_pseudonym.to_string(),
                my_subkey: invite.bob_subkey,
                participants,
                mek_generation: 0,
                slot_seed_hex: hex::encode(invite.slot_seed),
                wrapped_mek_blob: None,
                created_at: i64::try_from(rekindle_utils::timestamp_ms() / 1000)
                    .unwrap_or(i64::MAX),
            },
        )
        .await?;
    deps.emit_event(DmEvent::InviteReceived {
        record_key: invite.record_key.to_string(),
        sender_pseudonym: invite.alice_pseudonym.to_string(),
        sender_public_key_hex: invite.sender_hex.to_string(),
        is_group: false,
    });
    Ok(())
}

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
        is_group: true,
    });
    Ok(())
}

/// Handle an inbound `DmDecline` from a peer we invited. Removes the
/// pending invite row. (No DmEvent emitted — the frontend already
/// rendered the outbound state and will see the conversation drop on
/// next list refresh.)
pub async fn handle_incoming_dm_decline<D: DmDeps + ?Sized>(
    deps: &D,
    record_key: &str,
) -> Result<(), DmError> {
    let owner_key = deps.owner_key()?;
    deps.store().decline_invite(&owner_key, record_key).await
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
    deps.store().decline_invite(&owner_key, record_key).await
}
