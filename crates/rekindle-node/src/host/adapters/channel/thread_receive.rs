//! A thread's control messages from the mesh (plan E3.1 E3f), on both
//! modes: a `ThreadMessageReceived` from its authenticated sender stores
//! the reply that sender's verified outbox entry carries, opened from the
//! thread record exactly as the thread view reads it (the notice is only a
//! hint: its claimed author and fields are never stored).
//! The notification carries the sealed body but not the subkey and
//! Lamport position its AAD binds, so the reply is read from the record
//! once the write is readable there (the gossip can outrun the DHT write).
//! Our own reply is stored and published the same way at send (plan E7.13
//! M76): the mesh never echoes a frame to its sender, so without it the
//! author's own window never showed the reply (Signal, Discord: the
//! author's view changes at once).
//!
//! A thread's row and its archive state change only from signed
//! governance ops (`ThreadCreated` / `ThreadArchived` entries, merged and
//! projected by `governance/thread_events.rs`), never from an unsigned
//! gossip frame (threat model: unsigned frames never change state; plan
//! E3.2 S8-E).

use rekindle_channel::ThreadMessageView;
use rekindle_codec::community::envelope::ControlPayload;
use rekindle_db::repo::threads::{self, ThreadMessageRow};
use rekindle_lifecycle::SessionScope;
use rekindle_types::subscription_events::{SocialEvent, SubscriptionEvent};
use rekindle_types::thread::thread_reply_key;

use super::HostChannel;

/// The most replies one read of the thread record looks through.
const REPLY_SCAN: u32 = 200;

impl HostChannel {
    /// Store one thread control message of `community_id` from its
    /// authenticated `sender`; the event the shell shows, or `None` when it
    /// is no thread message, its row could not be stored, or no verified
    /// reply backs it. The envelope is only a hint (threat model: unsigned
    /// frames never change state): a reply is materialised only from the
    /// verified outbox entry for its `message_id` whose author is `sender`,
    /// and shows that author, body and commit time; an envelope naming
    /// another author, or one no readable entry of its sender backs, stores
    /// nothing and publishes nothing.
    pub async fn on_thread_control(
        &self,
        community_id: &str,
        sender: &str,
        payload: &ControlPayload,
        scope: &SessionScope,
    ) -> Option<SocialEvent> {
        let ControlPayload::ThreadMessageReceived {
            thread_id,
            message_id,
            sender_pseudonym: claimed,
            ..
        } = payload
        else {
            return None;
        };
        if !claimed.eq_ignore_ascii_case(sender) {
            tracing::debug!(
                community_id,
                "thread reply dropped: the notice names another author than its sender"
            );
            return None;
        }
        let (owner, db) = self.owner_and_db().ok()?;
        let Some(reply) = self
            .verified_reply(community_id, thread_id, message_id, sender, scope)
            .await
        else {
            tracing::debug!(
                community_id,
                "thread reply dropped: no readable verified entry of its sender backs it"
            );
            return None;
        };
        let event = SocialEvent::ThreadMessagePosted {
            community: community_id.to_string(),
            thread_id: thread_id.clone(),
            message_id: thread_reply_key(&reply.sender_pseudonym, message_id),
            sender_pseudonym: reply.sender_pseudonym,
            body: Some(reply.body),
            timestamp: reply.timestamp_ms / 1000,
            reply_to_id: None,
        };
        store_reply(&db, owner, message_id.clone(), &event).await?;
        Some(event)
    }

    /// Store and publish our own reply `sent` of `body` to `thread_id`,
    /// as [`on_thread_control`](Self::on_thread_control) does a received
    /// one. Without an unlocked identity or a stored row nothing is
    /// published: the reply is already written and sent.
    pub(crate) async fn publish_own_reply(
        &self,
        community_id: &str,
        thread_id: &str,
        body: &str,
        sent: rekindle_channel::ThreadReplySent,
    ) {
        let Ok((owner, db)) = self.owner_and_db() else {
            return;
        };
        let event = SocialEvent::ThreadMessagePosted {
            community: community_id.to_string(),
            thread_id: thread_id.to_string(),
            message_id: thread_reply_key(&sent.sender_pseudonym, &sent.message_id),
            sender_pseudonym: sent.sender_pseudonym,
            body: Some(body.to_string()),
            timestamp: sent.timestamp,
            reply_to_id: None,
        };
        if store_reply(&db, owner, sent.message_id, &event)
            .await
            .is_some()
        {
            self.ctx().publish_event(SubscriptionEvent::Social(event));
        }
    }

    /// The verified reply `message_id` of `sender` in `thread_id`, read
    /// from the thread record on the shared backoff until it is readable
    /// and opens (the notice can outrun the write or the key), or `None`
    /// once the retries end or the scope stops.
    async fn verified_reply(
        &self,
        community_id: &str,
        thread_id: &str,
        message_id: &str,
        sender: &str,
        scope: &SessionScope,
    ) -> Option<ThreadMessageView> {
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
                if let Some(reply) = sender_reply(messages, message_id, sender) {
                    return Some(reply);
                }
            }
            let wait = rekindle_records::retry::backoff_duration(attempt);
            stop.run_until_cancelled(tokio::time::sleep(wait)).await?;
        }
        None
    }
}

/// The reply `message_id` among the verified `replies` (each naming its
/// outbox's author) whose author is `sender` and whose body opened.
fn sender_reply(
    replies: Vec<ThreadMessageView>,
    message_id: &str,
    sender: &str,
) -> Option<ThreadMessageView> {
    replies.into_iter().find(|m| {
        m.server_message_id.as_deref() == Some(message_id)
            && m.sender_pseudonym.eq_ignore_ascii_case(sender)
            && !m.decryption_failed
            && m.admission.is_none()
    })
}

/// Store the reply under its verified author and the raw outbox id.
async fn store_reply(
    db: &rekindle_db::Db,
    owner: String,
    message_id: String,
    event: &SocialEvent,
) -> Option<()> {
    let SocialEvent::ThreadMessagePosted {
        community,
        thread_id,
        sender_pseudonym,
        body,
        timestamp,
        reply_to_id,
        ..
    } = event.clone()
    else {
        return None;
    };
    db.call(move |conn| {
        let row = ThreadMessageRow {
            thread_id: &thread_id,
            message_id: &message_id,
            sender_pseudonym: &sender_pseudonym,
            body: body.as_deref().unwrap_or_default(),
            timestamp,
            reply_to_id: reply_to_id.as_deref(),
        };
        threads::insert_message(conn, &owner, &community, &row)
    })
    .await
    .ok()
}

#[cfg(test)]
#[path = "thread_receive_tests.rs"]
mod tests;
