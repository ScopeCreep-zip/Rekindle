//! Inbound DM dispatch: persist invite, surface notification.
//!
//! Per architecture §27.1, a `DmInvite` arrives via `app_message`/`app_call`
//! after the friend has already created the SMPL record. Bob's job here
//! is to (1) persist the row in `dms` so the UI can render the invite,
//! (2) emit a `DirectMessageInvite` chat event so the frontend can
//! prompt accept/decline. Acceptance opens the SMPL record writable;
//! decline sends `DmDecline` and deletes the row.

use std::sync::Arc;

use rekindle_dm::GroupDmParticipant;
use tauri::Emitter;

use crate::channels::ChatEvent;
use crate::db::DbPool;
use crate::state::AppState;

#[allow(clippy::too_many_arguments)]
pub async fn handle_incoming_dm_invite(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    record_key: &str,
    slot_seed: &[u8],
    alice_pseudonym: &str,
    alice_subkey: u32,
    bob_subkey: u32,
) -> Result<(), String> {
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
    super::store::persist_dm_invite_pending(
        state,
        pool,
        record_key,
        false,
        sender_hex,
        alice_pseudonym,
        bob_subkey,
        &participants,
        0,
        &hex::encode(slot_seed),
        None,
    )
    .await?;
    let _ = app_handle.emit(
        "chat-event",
        &ChatEvent::DirectMessageInvite {
            from: sender_hex.to_string(),
            record_key: record_key.to_string(),
            initiator_pseudonym: alice_pseudonym.to_string(),
            is_group: false,
        },
    );
    Ok(())
}

pub async fn handle_incoming_dm_decline(
    state: &Arc<AppState>,
    pool: &DbPool,
    record_key: &str,
) -> Result<(), String> {
    super::store::decline_dm_invite(state, pool, record_key).await
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_incoming_group_dm_invite(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &DbPool,
    sender_hex: &str,
    record_key: &str,
    slot_seed: &[u8],
    initiator_pseudonym: &str,
    participants_json: &str,
    wrapped_mek: &[u8],
    mek_generation: u32,
) -> Result<(), String> {
    let participants: Vec<GroupDmParticipant> = serde_json::from_str(participants_json)
        .map_err(|e| format!("invalid participants_json: {e}"))?;
    // Architecture §27.2: each `GroupDmParticipant` carries a `public_key`
    // for verification. Bob finds *his* slot by matching against his own
    // identity public key — not against the sender, which is the
    // initiator and lives in their own slot.
    let my_pubkey_hex = {
        let secret = state.identity_secret.lock();
        let secret_bytes = (*secret).ok_or_else(|| "identity not unlocked".to_string())?;
        let identity = rekindle_crypto::Identity::from_secret_bytes(&secret_bytes);
        identity.public_key_hex()
    };
    let my_subkey = participants
        .iter()
        .find(|p| p.public_key.eq_ignore_ascii_case(&my_pubkey_hex))
        .ok_or_else(|| {
            "my pubkey not in group dm participants — invite not addressed to us".to_string()
        })?
        .subkey;
    super::store::persist_dm_invite_pending(
        state,
        pool,
        record_key,
        true,
        sender_hex,
        initiator_pseudonym,
        my_subkey,
        &participants,
        mek_generation,
        &hex::encode(slot_seed),
        Some(wrapped_mek),
    )
    .await?;
    let _ = app_handle.emit(
        "chat-event",
        &ChatEvent::DirectMessageInvite {
            from: sender_hex.to_string(),
            record_key: record_key.to_string(),
            initiator_pseudonym: initiator_pseudonym.to_string(),
            is_group: true,
        },
    );
    Ok(())
}

pub async fn handle_incoming_dm_leave(
    state: &Arc<AppState>,
    pool: &DbPool,
    _sender_hex: &str,
    record_key: &str,
) -> Result<(), String> {
    super::store::decline_dm_invite(state, pool, record_key).await
}
