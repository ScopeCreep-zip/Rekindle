use std::sync::Arc;

use crate::state::AppState;
use crate::state_helpers;
use rekindle_db::Db;

use super::control_event_records::{handle_event_payload, handle_game_server_payload};
use super::control_moderation::handle_gossip_control_payloads;
use crate::services::governance_adapter;

pub(crate) async fn handle_control_events_and_threads(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
    sender_pseudonym: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        ControlPayload::ChannelOverwriteChanged { channel_id } => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Governance(rekindle_types::subscription_events::GovernanceEvent::ChannelPermissionsChanged {
                    community: community_id.to_string(),
                    channel: channel_id,
                }),
            );
        }
        payload @ (ControlPayload::MessagePinned { .. }
        | ControlPayload::MessageUnpinned { .. }) => {
            handle_pin_payload(app_handle, state, pool, community_id, payload);
        }
        payload @ (ControlPayload::EventCreated { .. }
        | ControlPayload::EventUpdated { .. }
        | ControlPayload::EventDeleted { .. }
        | ControlPayload::EventRsvpChanged { .. }) => {
            handle_event_payload(app_handle, state, pool, community_id, payload);
        }
        payload @ (ControlPayload::ThreadCreated { .. }
        | ControlPayload::ThreadArchived { .. }
        | ControlPayload::ThreadMessageReceived { .. }) => {
            handle_thread_payload(app_handle, state, pool, community_id, payload);
        }
        payload @ (ControlPayload::GameServerAdded { .. }
        | ControlPayload::GameServerRemoved { .. }) => {
            handle_game_server_payload(app_handle, state, pool, community_id, payload);
        }
        ControlPayload::MEKRotated {
            channel_id,
            new_generation,
            ..
        } => {
            tracing::debug!(
                community = %community_id,
                "MEKRotated: v2.0 uses invite-time MEK distribution — vault read skipped"
            );
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Crypto(
                    rekindle_types::subscription_events::CryptoEvent::MekRotated {
                        community: community_id.to_string(),
                        channel: channel_id,
                        generation: new_generation,
                        // The gossip payload names no rotator.
                        rotator_pseudonym: None,
                    },
                ),
            );
        }
        ControlPayload::KickedNotification => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::System(
                    rekindle_types::subscription_events::SystemEvent::Kicked {
                        community: community_id.to_string(),
                    },
                ),
            );
        }
        ControlPayload::SubmitOnboardingAnswers { ref answers } => {
            governance_adapter::process_onboarding_answers(
                state,
                app_handle,
                community_id,
                sender_pseudonym,
                answers,
            )
            .await;
        }
        ControlPayload::OnboardingComplete {
            ref pseudonym_key,
            ref role_ids,
        } => {
            crate::event_dispatch::emit_membership(
                app_handle,
                rekindle_types::subscription_events::MembershipEvent::OnboardingCompleted {
                    community: community_id.to_string(),
                    pseudonym: pseudonym_key.clone(),
                    role_ids: role_ids.clone(),
                },
            );
        }
        ControlPayload::EventReminder {
            event_id,
            title,
            minutes_until_start,
        } => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Social(
                    rekindle_types::subscription_events::SocialEvent::EventReminder {
                        community: community_id.to_string(),
                        event_id,
                        title,
                        minutes_until_start,
                    },
                ),
            );
        }
        ControlPayload::SystemMessage { body, timestamp } => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::System(
                    rekindle_types::subscription_events::SystemEvent::Announcement {
                        // Tier 1's announcement is optionally global;
                        // a gossiped system message always has one.
                        community: Some(community_id.to_string()),
                        body,
                        timestamp,
                    },
                ),
            );
        }
        ControlPayload::RaidAlert { active } => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::System(
                    rekindle_types::subscription_events::SystemEvent::RaidAlert {
                        community: community_id.to_string(),
                        active,
                    },
                ),
            );
        }
        ControlPayload::ChannelLockdown { locked } => {
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::System(
                    rekindle_types::subscription_events::SystemEvent::ChannelLockdown {
                        community: community_id.to_string(),
                        locked,
                    },
                ),
            );
        }
        other => {
            handle_gossip_control_payloads(
                app_handle,
                state,
                community_id,
                sender_pseudonym,
                other,
            );
        }
    }
}

fn handle_pin_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        ControlPayload::MessagePinned {
            channel_id,
            message_id,
            pinned_by,
        } => {
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let ch = channel_id.clone();
            let mid = message_id.clone();
            let pb = pinned_by.clone();
            let now = rekindle_utils::timestamp_secs();
            crate::db_helpers::db_fire(pool, "persist pin", move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO channel_pins (owner_key, community_id, channel_id, message_id, pinned_by, pinned_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![owner_key, cid, ch, mid, pb, now],
                )?;
                Ok(())
            });
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Social(
                    rekindle_types::subscription_events::SocialEvent::MessagePinned {
                        community: community_id.to_string(),
                        channel: channel_id,
                        message_id,
                        pinned_by,
                    },
                ),
            );
        }
        ControlPayload::MessageUnpinned {
            channel_id,
            message_id,
        } => {
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let ch = channel_id.clone();
            let mid = message_id.clone();
            crate::db_helpers::db_fire(pool, "remove pin", move |conn| {
                conn.execute(
                    "DELETE FROM channel_pins WHERE owner_key = ?1 AND community_id = ?2 \
                     AND channel_id = ?3 AND message_id = ?4",
                    rusqlite::params![owner_key, cid, ch, mid],
                )?;
                Ok(())
            });
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Social(
                    rekindle_types::subscription_events::SocialEvent::MessageUnpinned {
                        community: community_id.to_string(),
                        channel: channel_id,
                        message_id,
                    },
                ),
            );
        }
        _ => {}
    }
}

fn handle_thread_payload(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    pool: &Db,
    community_id: &str,
    payload: rekindle_codec::community::envelope::ControlPayload,
) {
    use rekindle_codec::community::envelope::ControlPayload;

    match payload {
        ControlPayload::ThreadCreated { thread } => {
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let persisted = thread.clone();
            crate::db_helpers::db_fire(pool, "persist thread", move |conn| {
                conn.execute(
                    "INSERT OR REPLACE INTO community_threads \
                     (owner_key, community_id, id, channel_id, name, starter_message_id, \
                      creator_pseudonym, created_at, archived, auto_archive_seconds, \
                      last_message_at, message_count) \
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                    rusqlite::params![
                        owner_key,
                        cid,
                        persisted.id,
                        persisted.channel_id,
                        persisted.name,
                        persisted.starter_message_id,
                        persisted.creator_pseudonym,
                        persisted.created_at,
                        i32::from(persisted.archived),
                        persisted.auto_archive_seconds,
                        persisted.last_message_at,
                        persisted.message_count,
                    ],
                )?;
                Ok(())
            });
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Social(
                    rekindle_types::subscription_events::SocialEvent::ThreadCreated {
                        community: community_id.to_string(),
                        thread: Box::new(thread),
                    },
                ),
            );
        }
        ControlPayload::ThreadArchived {
            thread_id,
            archived,
        } => {
            let owner_key = state_helpers::current_owner_key(state).unwrap_or_default();
            let cid = community_id.to_string();
            let tid = thread_id.clone();
            let arch = archived;
            crate::db_helpers::db_fire(pool, "update thread archived", move |conn| {
                conn.execute(
                    "UPDATE community_threads SET archived = ?1 \
                     WHERE owner_key = ?2 AND community_id = ?3 AND id = ?4",
                    rusqlite::params![i32::from(arch), owner_key, cid, tid],
                )?;
                Ok(())
            });
            crate::event_dispatch::emit_subscription(
                app_handle,
                &rekindle_types::subscription_events::SubscriptionEvent::Social(
                    rekindle_types::subscription_events::SocialEvent::ThreadArchiveChanged {
                        community: community_id.to_string(),
                        thread_id,
                        archived,
                    },
                ),
            );
        }
        ControlPayload::ThreadMessageReceived {
            thread_id,
            message_id,
            sender_pseudonym,
            ciphertext: _,
            mek_generation: _,
            timestamp,
            reply_to_id,
        } => {
            // The notification carries the sealed body but not the subkey
            // and Lamport position its AAD binds, so the body is opened
            // from the thread record — exactly as the thread view reads it
            // — once the write is readable there.
            let app_handle = app_handle.clone();
            let state = Arc::clone(state);
            let pool = pool.clone();
            let community_id = community_id.to_string();
            crate::state_helpers::login_scope_or_closed(&state).spawn_or_drop("thread payload fetch", async move {
                let body =
                    fetch_thread_reply_body(&state, &community_id, &thread_id, &message_id).await;
                let owner_key = state_helpers::current_owner_key(&state).unwrap_or_default();
                let cid = community_id.clone();
                let tid = thread_id.clone();
                let mid = message_id.clone();
                let sp = sender_pseudonym.clone();
                let persisted_body = body.clone().unwrap_or_default();
                let rid = reply_to_id.clone();
                crate::db_helpers::db_fire(&pool, "persist thread message", move |conn| {
                    conn.execute(
                        "INSERT OR IGNORE INTO thread_messages \
                         (owner_key, community_id, thread_id, message_id, sender_pseudonym, body, timestamp, reply_to_id) \
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                        rusqlite::params![owner_key, cid, tid, mid, sp, persisted_body, timestamp, rid],
                    )?;
                    conn.execute(
                        "UPDATE community_threads SET message_count = message_count + 1, last_message_at = ?1 \
                         WHERE owner_key = ?2 AND community_id = ?3 AND id = ?4",
                        rusqlite::params![timestamp, owner_key, cid, tid],
                    )?;
                    Ok(())
                });
                crate::event_dispatch::emit_subscription(
                    &app_handle,
                    &rekindle_types::subscription_events::SubscriptionEvent::Social(
                        rekindle_types::subscription_events::SocialEvent::ThreadMessagePosted {
                            community: community_id,
                            thread_id,
                            message_id,
                            sender_pseudonym,
                            body,
                            timestamp,
                            reply_to_id,
                        },
                    ),
                );
            });
        }
        _ => {}
    }
}

/// A thread reply's body, read from the thread record under its exact
/// key generation and AAD position. The gossip can outrun the DHT write,
/// so the read is retried on the shared backoff; `None` when the reply
/// never becomes readable or does not open.
async fn fetch_thread_reply_body(
    state: &Arc<AppState>,
    community_id: &str,
    thread_id: &str,
    message_id: &str,
) -> Option<String> {
    for attempt in 0..rekindle_records::retry::MAX_RETRIES {
        if let Ok(messages) = crate::services::community::threads::load_thread_messages(
            state,
            community_id,
            thread_id,
            200,
            None,
        )
        .await
        {
            if let Some(message) = messages
                .into_iter()
                .find(|m| m.server_message_id.as_deref() == Some(message_id))
            {
                return (!message.decryption_failed).then_some(message.body);
            }
        }
        tokio::time::sleep(rekindle_records::retry::backoff_duration(attempt)).await;
    }
    None
}
