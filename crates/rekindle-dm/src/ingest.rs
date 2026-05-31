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

/// Handle a 1:1 DM invite from `sender_hex` for the SMPL record
/// allocated at `record_key`. Persists locally; emits an invite event.
#[allow(
    clippy::too_many_arguments,
    reason = "DmInvite wire envelope has 6 distinct fields; bundling would only move the args from call site to constructor"
)]
pub async fn handle_incoming_dm_invite<D: DmDeps + ?Sized>(
    deps: &D,
    sender_hex: &str,
    record_key: &str,
    slot_seed: &[u8],
    alice_pseudonym: &str,
    alice_subkey: u32,
    bob_subkey: u32,
) -> Result<(), DmError> {
    let owner_key = deps.owner_key()?;
    let participants = vec![
        GroupDmParticipant {
            pseudonym: alice_pseudonym.to_string(),
            subkey: alice_subkey,
            public_key: sender_hex.to_string(),
        },
        // Our slot — public_key filled in when we accept and derive.
        GroupDmParticipant {
            pseudonym: String::new(),
            subkey: bob_subkey,
            public_key: String::new(),
        },
    ];
    deps.store()
        .persist_invite_pending(
            &owner_key,
            DmInvitePending {
                record_key: record_key.to_string(),
                is_group: false,
                initiator_public_key: sender_hex.to_string(),
                initiator_pseudonym: alice_pseudonym.to_string(),
                my_subkey: bob_subkey,
                participants,
                mek_generation: 0,
                slot_seed_hex: hex::encode(slot_seed),
                wrapped_mek_blob: None,
                created_at: i64::try_from(rekindle_utils::timestamp_ms() / 1000)
                    .unwrap_or(i64::MAX),
            },
        )
        .await?;
    deps.emit_event(DmEvent::InviteReceived {
        record_key: record_key.to_string(),
        sender_pseudonym: alice_pseudonym.to_string(),
        sender_public_key_hex: sender_hex.to_string(),
        is_group: false,
    });
    Ok(())
}

/// Handle a group DM invite. Architecture §27.2: each `GroupDmParticipant`
/// carries a `public_key` for verification — we find OUR slot by
/// matching against our own identity public key.
#[allow(
    clippy::too_many_arguments,
    reason = "GroupDmInvite wire envelope has 8 distinct fields; bundling would only move the args from call site to constructor"
)]
pub async fn handle_incoming_group_dm_invite<D: DmDeps + ?Sized>(
    deps: &D,
    sender_hex: &str,
    record_key: &str,
    slot_seed: &[u8],
    initiator_pseudonym: &str,
    participants_json: &str,
    wrapped_mek: &[u8],
    mek_generation: u32,
) -> Result<(), DmError> {
    let owner_key = deps.owner_key()?;
    let participants: Vec<GroupDmParticipant> = serde_json::from_str(participants_json)
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
                record_key: record_key.to_string(),
                is_group: true,
                initiator_public_key: sender_hex.to_string(),
                initiator_pseudonym: initiator_pseudonym.to_string(),
                my_subkey,
                participants,
                mek_generation,
                slot_seed_hex: hex::encode(slot_seed),
                wrapped_mek_blob: Some(wrapped_mek.to_vec()),
                created_at: i64::try_from(rekindle_utils::timestamp_ms() / 1000)
                    .unwrap_or(i64::MAX),
            },
        )
        .await?;
    deps.emit_event(DmEvent::InviteReceived {
        record_key: record_key.to_string(),
        sender_pseudonym: initiator_pseudonym.to_string(),
        sender_public_key_hex: sender_hex.to_string(),
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
