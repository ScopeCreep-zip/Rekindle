//! Local SQLite persistence for DM conversations and messages.

use std::sync::Arc;

use rekindle_dm::GroupDmParticipant;
use serde::{Deserialize, Serialize};

use crate::db::DbPool;
use crate::db_helpers::{db_call, db_call_or_default};
use crate::state::AppState;
use crate::state_helpers;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DmConversation {
    pub record_key: String,
    pub is_group: bool,
    pub initiator_public_key: String,
    pub initiator_pseudonym: String,
    pub my_subkey: u32,
    pub participants: Vec<GroupDmParticipant>,
    pub mek_generation: u32,
    pub created_at: i64,
    pub last_message_at: Option<i64>,
}

#[allow(clippy::too_many_arguments)]
pub async fn persist_dm_invite_pending(
    state: &Arc<AppState>,
    pool: &DbPool,
    record_key: &str,
    is_group: bool,
    initiator_public_key: &str,
    initiator_pseudonym: &str,
    my_subkey: u32,
    participants: &[GroupDmParticipant],
    mek_generation: u32,
    slot_seed_hex: &str,
    wrapped_mek_blob: Option<&[u8]>,
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }
    let participants_json = serde_json::to_string(participants)
        .map_err(|e| format!("serialize dm participants: {e}"))?;
    let record = record_key.to_string();
    let initiator_pk = initiator_public_key.to_string();
    let initiator_ps = initiator_pseudonym.to_string();
    let seed = slot_seed_hex.to_string();
    let wrapped: Option<Vec<u8>> = wrapped_mek_blob.map(<[u8]>::to_vec);
    let now = crate::db::timestamp_now();
    let group_flag = i64::from(is_group);
    let my_subkey_i = i64::from(my_subkey);
    let gen_i = i64::from(mek_generation);
    db_call(pool, move |conn| {
        conn.execute(
            "INSERT INTO dms
                (owner_key, record_key, is_group, initiator_public_key, initiator_pseudonym,
                 my_subkey, participants_json, slot_seed_hex, wrapped_mek_blob,
                 mek_generation, created_at, last_message_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL)
             ON CONFLICT(owner_key, record_key) DO NOTHING",
            rusqlite::params![
                owner_key,
                record,
                group_flag,
                initiator_pk,
                initiator_ps,
                my_subkey_i,
                participants_json,
                seed,
                wrapped,
                gen_i,
                now
            ],
        )?;
        Ok(())
    })
    .await
}

pub async fn list_dm_conversations(
    state: &Arc<AppState>,
    pool: &DbPool,
) -> Vec<DmConversation> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Vec::new();
    }
    db_call_or_default(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT record_key, is_group, initiator_public_key, initiator_pseudonym,
                    my_subkey, participants_json, mek_generation, created_at, last_message_at
             FROM dms WHERE owner_key = ?1 ORDER BY COALESCE(last_message_at, created_at) DESC",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![owner_key], |row| {
                let participants_json: String = row.get(5)?;
                let participants: Vec<GroupDmParticipant> =
                    serde_json::from_str(&participants_json).unwrap_or_default();
                Ok(DmConversation {
                    record_key: row.get(0)?,
                    is_group: {
                        let n: i64 = row.get(1)?;
                        n != 0
                    },
                    initiator_public_key: row.get(2)?,
                    initiator_pseudonym: row.get(3)?,
                    my_subkey: {
                        let n: i64 = row.get(4)?;
                        u32::try_from(n).unwrap_or(0)
                    },
                    participants,
                    mek_generation: {
                        let n: i64 = row.get(6)?;
                        u32::try_from(n).unwrap_or(0)
                    },
                    created_at: row.get(7)?,
                    last_message_at: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
}

/// One persisted DM message row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DmMessageRecord {
    pub id: i64,
    pub sender_pseudonym: String,
    pub body: String,
    pub timestamp: i64,
    pub sequence: i64,
    pub mek_generation: i64,
}

/// Read recent messages for a DM conversation, oldest-first.
pub async fn load_dm_messages(
    state: &Arc<AppState>,
    pool: &DbPool,
    record_key: &str,
    limit: i64,
) -> Vec<DmMessageRecord> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Vec::new();
    }
    let owner = owner_key;
    let record = record_key.to_string();
    db_call_or_default(pool, move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, sender_pseudonym, body, timestamp, sequence, mek_generation
             FROM dm_messages
             WHERE owner_key = ?1 AND record_key = ?2
             ORDER BY timestamp DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(rusqlite::params![owner, record, limit], |row| {
            Ok(DmMessageRecord {
                id: row.get(0)?,
                sender_pseudonym: row.get(1)?,
                body: row.get(2)?,
                timestamp: row.get(3)?,
                sequence: row.get(4)?,
                mek_generation: row.get(5)?,
            })
        })?;
        let mut collected: Vec<DmMessageRecord> = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        collected.reverse(); // oldest-first for the UI scrollback
        Ok(collected)
    })
    .await
}

pub async fn decline_dm_invite(
    state: &Arc<AppState>,
    pool: &DbPool,
    record_key: &str,
) -> Result<(), String> {
    let owner_key = state_helpers::owner_key_or_default(state);
    if owner_key.is_empty() {
        return Err("no identity".into());
    }
    let record = record_key.to_string();
    db_call(pool, move |conn| {
        conn.execute(
            "DELETE FROM dms WHERE owner_key = ?1 AND record_key = ?2",
            rusqlite::params![owner_key, record],
        )?;
        Ok(())
    })
    .await
}
