//! A thread's control messages from the mesh (plan E3.1 E3f), on both
//! modes: a `ThreadCreated` stores the thread's row, a `ThreadArchived`
//! its archive state, and a `ThreadMessageReceived` the reply, whose body
//! is opened from the thread record exactly as the thread view reads it.
//! The notification carries the sealed body but not the subkey and
//! Lamport position its AAD binds, so the reply is read from the record
//! once the write is readable there (the gossip can outrun the DHT write).

use rekindle_codec::community::envelope::ControlPayload;
use rekindle_db::repo::threads::{self, ThreadMessageRow};
use rekindle_lifecycle::SessionScope;
use rekindle_types::subscription_events::SocialEvent;

use super::HostChannel;

/// The most replies one read of the thread record looks through.
const REPLY_SCAN: u32 = 200;

impl HostChannel {
    /// Store one thread control message of `community_id`; the event the
    /// shell shows, or `None` when it is no thread message or its row could
    /// not be stored. A reply's body is `None` when it never became
    /// readable or did not open.
    pub async fn on_thread_control(
        &self,
        community_id: &str,
        payload: &ControlPayload,
        scope: &SessionScope,
    ) -> Option<SocialEvent> {
        let (owner, db) = self.owner_and_db().ok()?;
        let community = community_id.to_string();
        let event = match payload {
            ControlPayload::ThreadCreated { thread } => {
                let (c, row) = (community.clone(), thread.clone());
                db.call(move |conn| threads::upsert(conn, &owner, &c, &row))
                    .await
                    .ok()?;
                SocialEvent::ThreadCreated {
                    community,
                    thread: Box::new(thread.clone()),
                }
            }
            ControlPayload::ThreadArchived {
                thread_id,
                archived,
            } => {
                let (c, t, a) = (community.clone(), thread_id.clone(), *archived);
                db.call(move |conn| threads::set_archived(conn, &owner, &c, &t, a))
                    .await
                    .ok()?;
                SocialEvent::ThreadArchiveChanged {
                    community,
                    thread_id: thread_id.clone(),
                    archived: *archived,
                }
            }
            ControlPayload::ThreadMessageReceived {
                thread_id,
                message_id,
                sender_pseudonym,
                timestamp,
                reply_to_id,
                ..
            } => {
                let body = self
                    .reply_body(community_id, thread_id, message_id, scope)
                    .await;
                let (c, t, m, s, b, r) = (
                    community.clone(),
                    thread_id.clone(),
                    message_id.clone(),
                    sender_pseudonym.clone(),
                    body.clone().unwrap_or_default(),
                    reply_to_id.clone(),
                );
                let ts = *timestamp;
                db.call(move |conn| {
                    let row = ThreadMessageRow {
                        thread_id: &t,
                        message_id: &m,
                        sender_pseudonym: &s,
                        body: &b,
                        timestamp: ts,
                        reply_to_id: r.as_deref(),
                    };
                    threads::insert_message(conn, &owner, &c, &row)
                })
                .await
                .ok()?;
                SocialEvent::ThreadMessagePosted {
                    community,
                    thread_id: thread_id.clone(),
                    message_id: message_id.clone(),
                    sender_pseudonym: sender_pseudonym.clone(),
                    body,
                    timestamp: *timestamp,
                    reply_to_id: reply_to_id.clone(),
                }
            }
            _ => return None,
        };
        Some(event)
    }

    /// The body of reply `message_id`, read from the thread record on the
    /// shared backoff until it is readable or the scope stops.
    async fn reply_body(
        &self,
        community_id: &str,
        thread_id: &str,
        message_id: &str,
        scope: &SessionScope,
    ) -> Option<String> {
        let stop = scope.token();
        for attempt in 0..rekindle_records::retry::MAX_RETRIES {
            if let Ok(messages) = rekindle_channel::load_thread_messages(
                self,
                community_id,
                thread_id,
                REPLY_SCAN,
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
            let wait = rekindle_records::retry::backoff_duration(attempt);
            stop.run_until_cancelled(tokio::time::sleep(wait)).await?;
        }
        None
    }
}
