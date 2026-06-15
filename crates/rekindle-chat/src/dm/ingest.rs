//! Inbound DM dispatch: invite, decline, leave, group invite.
//!
//! Called when an `InboundEvent::Call` carries a DM invite payload, or
//! when the inbox scanner discovers a DM invite entry. Persists the
//! conversation row and emits `DmEvent::InviteReceived` so the UI
//! can prompt accept/decline.

use rekindle_types::dm_store::{DmInvitePending, DmParticipant};

use crate::dm::deps::{DmDeps, DmEvent};
use crate::dm::error::DmError;
use crate::dm::invite::{DmInvite, GroupDmInvite};

/// Handle a 1:1 DM invite. Persists locally; emits invite event.
pub async fn handle_incoming_dm_invite(
    deps: &dyn DmDeps,
    sender_hex: &str,
    invite: &DmInvite,
) -> Result<(), DmError> {
    tracing::info!(
        record_key = &invite.record_key[..20.min(invite.record_key.len())],
        sender = &sender_hex[..16.min(sender_hex.len())],
        "dm::ingest: incoming 1:1 DM invite"
    );

    // Fill responder's public_key so the participants list is complete.
    // Without this, dm_get_session_meta cannot compute peer_public_key
    // for the responder.
    let my_pub_hex = deps.identity_public_key_hex()?;

    let participants = vec![
        DmParticipant {
            pseudonym: invite.initiator_pseudonym.clone(),
            subkey: invite.initiator_subkey,
            public_key: sender_hex.into(),
        },
        DmParticipant {
            pseudonym: String::new(),
            subkey: invite.responder_subkey,
            public_key: my_pub_hex,
        },
    ];

    deps.store().dm_persist_invite(DmInvitePending {
        record_key: invite.record_key.clone(),
        is_group: false,
        initiator_public_key: sender_hex.into(),
        initiator_pseudonym: invite.initiator_pseudonym.clone(),
        my_subkey: invite.responder_subkey,
        participants,
        mek_generation: 0,
        slot_seed_hex: hex::encode(&invite.slot_seed),
        wrapped_mek_blob: None,
    })?;

    deps.emit_event(DmEvent::InviteReceived {
        record_key: invite.record_key.clone(),
        sender_pseudonym: invite.initiator_pseudonym.clone(),
        sender_public_key_hex: sender_hex.into(),
        is_group: false,
    });

    tracing::info!(
        record_key = &invite.record_key[..12.min(invite.record_key.len())],
        "dm::ingest: 1:1 invite persisted and emitted"
    );
    Ok(())
}

/// Handle a group DM invite. Finds our slot by matching our public key
/// in the participant list. Persists locally; emits invite event.
pub async fn handle_incoming_group_dm_invite(
    deps: &dyn DmDeps,
    sender_hex: &str,
    invite: &GroupDmInvite,
) -> Result<(), DmError> {
    let my_pub_hex = deps.identity_public_key_hex()?;

    let my_subkey = invite
        .participants
        .iter()
        .find(|p| p.public_key.eq_ignore_ascii_case(&my_pub_hex))
        .ok_or_else(|| {
            DmError::InvalidInput(
                "my pubkey not in group dm participants — invite not addressed to us".into(),
            )
        })?
        .subkey;

    tracing::info!(
        record_key = &invite.record_key[..20.min(invite.record_key.len())],
        sender = &sender_hex[..16.min(sender_hex.len())],
        my_subkey,
        participant_count = invite.participants.len(),
        "dm::ingest: incoming group DM invite"
    );

    deps.store().dm_persist_invite(DmInvitePending {
        record_key: invite.record_key.clone(),
        is_group: true,
        initiator_public_key: sender_hex.into(),
        initiator_pseudonym: invite.initiator_pseudonym.clone(),
        my_subkey,
        participants: invite.participants.clone(),
        mek_generation: invite.mek_generation,
        slot_seed_hex: hex::encode(&invite.slot_seed),
        wrapped_mek_blob: Some(invite.wrapped_mek.clone()),
    })?;

    deps.emit_event(DmEvent::InviteReceived {
        record_key: invite.record_key.clone(),
        sender_pseudonym: invite.initiator_pseudonym.clone(),
        sender_public_key_hex: sender_hex.into(),
        is_group: true,
    });

    tracing::info!(
        record_key = &invite.record_key[..12.min(invite.record_key.len())],
        "dm::ingest: group invite persisted and emitted"
    );
    Ok(())
}

/// Handle a DM decline from a peer we invited.
pub async fn handle_incoming_dm_decline(
    deps: &dyn DmDeps,
    record_key: &str,
) -> Result<(), DmError> {
    tracing::info!(
        record_key = &record_key[..20.min(record_key.len())],
        "dm::ingest: peer declined DM invite"
    );
    deps.store().dm_decline_invite(record_key)?;
    deps.emit_event(DmEvent::InviteDeclined {
        record_key: record_key.into(),
        reason: String::new(),
    });
    Ok(())
}

/// Handle a group DM leave from a peer.
pub async fn handle_incoming_dm_leave(
    deps: &dyn DmDeps,
    sender_hex: &str,
    record_key: &str,
) -> Result<(), DmError> {
    tracing::info!(
        record_key = &record_key[..20.min(record_key.len())],
        sender = &sender_hex[..16.min(sender_hex.len())],
        "dm::ingest: peer left group DM"
    );
    deps.emit_event(DmEvent::GroupMemberLeft {
        record_key: record_key.into(),
        sender_public_key_hex: sender_hex.into(),
    });
    Ok(())
}
