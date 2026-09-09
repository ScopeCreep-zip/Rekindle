use std::sync::Arc;

use crate::db::DbPool;
use crate::db_helpers::db_fire;
use crate::state::AppState;
use tauri::Manager;

pub(crate) fn check_gossip_moderation_permission(
    state: &Arc<AppState>,
    community_id: &str,
    sender_pseudonym: &str,
    payload: &rekindle_protocol::dht::community::envelope::ControlPayload,
) -> bool {
    use rekindle_protocol::dht::community::envelope::ControlPayload;
    use rekindle_types::permissions;

    let required: u64 = match payload {
        ControlPayload::Kick { .. } => permissions::KICK_MEMBERS,
        ControlPayload::Ban { .. } | ControlPayload::Unban { .. } => permissions::BAN_MEMBERS,
        ControlPayload::TimeoutMember { .. } | ControlPayload::RemoveTimeout { .. } => {
            permissions::TIMEOUT_MEMBERS
        }
        _ => return true,
    };

    let Some(gov) = crate::state_helpers::governance_state(state, community_id) else {
        return false;
    };
    let Ok(bytes) = hex::decode(sender_pseudonym) else {
        return false;
    };
    let Ok(pseudo_bytes): Result<[u8; 32], _> = bytes.try_into() else {
        return false;
    };
    let perms = rekindle_governance::permissions::compute_permissions(
        &rekindle_types::id::PseudonymKey(pseudo_bytes),
        None,
        &gov,
        rekindle_utils::timestamp_secs(),
    );
    if rekindle_governance::permissions::has_capability(perms, required) {
        true
    } else {
        tracing::warn!(
            community = %community_id,
            sender = %sender_pseudonym,
            required = format!("{required:#x}"),
            "gossip moderation: sender lacks required permission — ignoring"
        );
        false
    }
}

/// Ceiling for one `SyncResponse` payload.
///
/// veilid-core rejects an `app_message` larger than
/// `MAX_APP_MESSAGE_MESSAGE_LEN` (32 768) with
/// `RPCError::internal("AppMessage message too long to set")` — it does
/// not truncate, so an oversize response is not a degraded reply, it is
/// no reply at all. The margin below the cap covers the
/// `CommunityEnvelope` wrapper, the signature, and the Cap'n Proto
/// framing around the message list.
///
/// The row limit alone was never enough: `SyncedMessage.sender_key` is a
/// 64-char hex pseudonym, so 500 rows is 32 000 bytes of sender keys
/// before a single message body.
const SYNC_RESPONSE_BUDGET_BYTES: usize = 28 * 1024;

/// Take messages oldest-first until the next one would not fit.
///
/// Oldest-first because the requester is filling a gap forward from
/// `since_timestamp`; truncating from the newest end would leave a hole
/// in the middle that no later request asks for again.
fn fit_within_budget(
    messages: Vec<rekindle_types::message::SyncedMessage>,
) -> Vec<rekindle_types::message::SyncedMessage> {
    let mut out = Vec::with_capacity(messages.len());
    // Start at 2 for the enclosing `[]`. Each row after the first also
    // costs a separator — summing row sizes alone under-counts the
    // encoded array, which is how the first version of this function
    // produced a payload 19 bytes over its own budget.
    let mut used = 2usize;
    for m in messages {
        // Measured, not estimated: serialising the row is cheap next to
        // being wrong about its size and losing the whole response.
        let size = serde_json::to_vec(&m).map_or(usize::MAX, |v| v.len());
        let separator = usize::from(!out.is_empty());
        if used.saturating_add(size).saturating_add(separator) > SYNC_RESPONSE_BUDGET_BYTES {
            break;
        }
        used += size + separator;
        out.push(m);
    }
    out
}

pub(crate) fn handle_sync_request(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    requester_pseudonym: &str,
    channel_id: &str,
    since_timestamp: u64,
) {
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();
    let community_id_owned = community_id.to_string();
    let channel_id_owned = channel_id.to_string();
    let channel_id_for_envelope = channel_id.to_string();
    let since_ts = since_timestamp.cast_signed();
    let requester_owned = requester_pseudonym.to_string();
    let state = Arc::clone(state);
    let pool = pool.inner().clone();

    tokio::spawn(async move {
        let messages: Vec<rekindle_types::message::SyncedMessage> =
            crate::db_helpers::db_call(&pool, move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT sender_key, body, timestamp, mek_generation, lamport_ts \
                         FROM messages \
                         WHERE owner_key = ? AND conversation_id = ? \
                           AND conversation_type = 'channel' AND timestamp >= ? \
                         ORDER BY timestamp ASC LIMIT 500",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![owner_key, channel_id_owned, since_ts],
                    |row| {
                        Ok(rekindle_types::message::SyncedMessage {
                            sender_key: row.get::<_, String>(0)?,
                            body: row.get::<_, String>(1)?,
                            timestamp: row.get::<_, i64>(2)?,
                            mek_generation: row.get::<_, Option<i64>>(3)?,
                            lamport_ts: row.get::<_, Option<i64>>(4)?,
                        })
                    },
                )?;
                Ok(rows.filter_map(std::result::Result::ok).collect::<Vec<_>>())
            })
            .await
            .unwrap_or_default();

        if messages.is_empty() {
            return;
        }

        let total = messages.len();
        let messages = fit_within_budget(messages);
        if messages.is_empty() {
            tracing::warn!(
                community = %community_id_owned,
                "first sync row alone exceeds the app_message budget — nothing sent"
            );
            return;
        }
        if messages.len() < total {
            tracing::debug!(
                community = %community_id_owned,
                sent = messages.len(),
                withheld = total - messages.len(),
                "sync response truncated to fit the app_message ceiling"
            );
        }

        tracing::debug!(
            community = %community_id_owned,
            count = messages.len(),
            "responding to sync request"
        );

        let envelope = rekindle_protocol::dht::community::envelope::CommunityEnvelope::Control(
            rekindle_protocol::dht::community::envelope::ControlPayload::SyncResponse {
                channel_id: channel_id_for_envelope,
                messages,
            },
        );
        // Directed, not broadcast: the requester asked, so the reply goes
        // to the requester. Broadcasting made every member re-receive
        // history they already held and multiplied an already-oversize
        // payload by the fan-out degree.
        if let Err(e) = crate::services::community::gossip::send_to_member(
            &state,
            &pool,
            &community_id_owned,
            &requester_owned,
            &envelope,
        )
        .await
        {
            tracing::warn!(
                community = %community_id_owned,
                requester = %requester_owned,
                error = %e,
                "sync response delivery failed"
            );
        }
    });
}

pub(crate) fn handle_sync_response(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    messages: &[rekindle_types::message::SyncedMessage],
) {
    if messages.is_empty() {
        return;
    }

    tracing::info!(
        community = %community_id,
        channel = %channel_id,
        count = messages.len(),
        "merging sync response messages"
    );

    let pool: tauri::State<'_, DbPool> = app_handle.state();
    let owner_key = crate::state_helpers::current_owner_key(state).unwrap_or_default();

    for message in messages {
        let sender = message.sender_key.clone();
        let body = message.body.clone();
        let timestamp = message.timestamp;
        let mek_generation = message.mek_generation;
        let owner_key = owner_key.clone();
        let channel_id = channel_id.to_string();
        db_fire(pool.inner(), "store sync message", move |conn| {
            conn.execute(
                "INSERT OR IGNORE INTO messages \
                 (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, is_read, mek_generation) \
                 VALUES (?, ?, 'channel', ?, ?, ?, 0, ?)",
                rusqlite::params![owner_key, channel_id, sender, body, timestamp, mek_generation],
            )?;
            Ok(())
        });
    }

    crate::event_dispatch::emit_subscription(
        app_handle,
        &rekindle_types::subscription_events::SubscriptionEvent::System(
            rekindle_types::subscription_events::SystemEvent::SyncReceived {
                community: community_id.to_string(),
                channel: channel_id.to_string(),
                message_count: messages.len(),
            },
        ),
    );
}

#[cfg(test)]
mod budget_tests {
    use super::{fit_within_budget, SYNC_RESPONSE_BUDGET_BYTES};
    use rekindle_types::message::SyncedMessage;

    /// Realistic row: a 64-char hex pseudonym and a 200-byte body.
    fn row(i: usize) -> SyncedMessage {
        SyncedMessage {
            sender_key: "a".repeat(64),
            body: format!("{i:0>200}"),
            timestamp: 1_710_000_000 + i64::try_from(i).unwrap_or(0),
            mek_generation: Some(1),
            lamport_ts: Some(i64::try_from(i).unwrap_or(0)),
        }
    }

    /// The defect this guards: `LIMIT 500` alone is a row bound, not a
    /// byte bound. 500 rows of 64-char sender keys is 32 000 bytes of
    /// keys before any body, and veilid-core rejects an `app_message`
    /// over 32 768 outright rather than truncating — so the whole
    /// history reply used to vanish.
    #[test]
    fn stops_before_the_app_message_ceiling() {
        let msgs: Vec<SyncedMessage> = (0..500).map(row).collect();
        let fitted = fit_within_budget(msgs);

        assert!(fitted.len() < 500, "500 realistic rows must not all fit");
        let encoded = serde_json::to_vec(&fitted).expect("encode");
        assert!(
            encoded.len() <= SYNC_RESPONSE_BUDGET_BYTES,
            "fitted response is {} bytes, over the {SYNC_RESPONSE_BUDGET_BYTES} budget",
            encoded.len()
        );
    }

    #[test]
    fn keeps_everything_when_it_fits() {
        let msgs: Vec<SyncedMessage> = (0..5).map(row).collect();
        assert_eq!(fit_within_budget(msgs).len(), 5);
    }

    /// Oldest-first: the requester is filling a gap forward from
    /// `since_timestamp`, so a truncated page must be a prefix. Dropping
    /// from the front would leave a hole nothing asks for again.
    #[test]
    fn truncates_from_the_newest_end() {
        let msgs: Vec<SyncedMessage> = (0..500).map(row).collect();
        let fitted = fit_within_budget(msgs.clone());
        assert!(!fitted.is_empty());
        for (i, m) in fitted.iter().enumerate() {
            assert_eq!(m.timestamp, msgs[i].timestamp, "row {i} is not a prefix");
        }
    }
}
