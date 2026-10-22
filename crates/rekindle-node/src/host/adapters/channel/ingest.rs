//! The one ingest path for received channel posts (plan E3.1 F3, D26), on
//! both modes: the daemon's gossip receive and the desktop's notification
//! receive both hand a post here. It opens the body, runs the
//! `rekindle_channel::ingest` gate, stores the row (with `mentions_me`, and
//! read when we wrote it from another device), publishes
//! `ChannelMessageEvent::New` and the channel's `UnreadChanged`.

use rekindle_channel::error::ChannelError;
use rekindle_channel::ingest::{self, BlockList, GateItem, GateReason, Verdict};
use rekindle_channel::ChannelMessagingDeps;
use rekindle_codec::community::channel_record::{ChannelMessage, ChannelRecordEntry};
use rekindle_db::repo::messages::channel::{self as rows, ChannelMessageInsert};
use rekindle_types::governance::{GovernanceEntry, ModReportTarget};
use rekindle_types::id::ChannelId;
use rekindle_types::message::SendState;
use rekindle_types::permissions::MANAGE_MESSAGES;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent, SystemEvent};

use super::HostChannel;

fn stored(e: impl std::fmt::Display) -> ChannelError {
    ChannelError::Adapter(format!("store: {e}"))
}

/// A received post: where it was written and what it holds.
#[derive(Debug, Clone)]
pub struct ReceivedPost {
    pub community_id: String,
    pub channel_id: String,
    pub record_key: String,
    pub subkey_index: u32,
    pub sequence: u64,
    pub message: ChannelMessage,
    /// The original author, when the post is a forward.
    pub forwarded_from_author: Option<String>,
    /// The attachment's stored record, as JSON.
    pub attachment_json: Option<String>,
}

/// The columns of a received row the ingest decides.
struct Row {
    message_id: String,
    body: String,
    blurred: bool,
    own: bool,
    mentions_me: bool,
}

/// What the ingest did with a post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ingested {
    /// The row was stored and published; `blurred` when the gate hid it.
    Stored { body: String, blurred: bool },
    /// We already hold it.
    Duplicate,
    /// No key we hold opens it: it is not stored, so its row never keeps an
    /// empty body.
    Undecryptable,
    /// The gate dropped it.
    Dropped(GateReason),
}

impl HostChannel {
    /// Fetch the post `message_id` by `author` from `subkey_index` of
    /// `channel_id`'s record (a gossip notification names all three) and
    /// ingest it.
    ///
    /// # Errors
    /// No identity is unlocked, the channel has no record, the read failed
    /// or the slot does not hold the message.
    pub async fn ingest_notified(
        &self,
        community_id: &str,
        channel_id: &str,
        subkey_index: u32,
        message_id: &str,
        author: &str,
        sequence: u64,
    ) -> Result<Ingested, ChannelError> {
        let record_key = self
            .channel_record_key(community_id, channel_id)
            .ok_or_else(|| ChannelError::Adapter(format!("channel {channel_id} has no record")))?;
        let value = (self
            .records()?
            .read_once(&record_key, subkey_index, true)
            .await)
            .map_err(|e| ChannelError::Adapter(format!("read channel slot: {e}")))?
            .ok_or_else(|| ChannelError::Adapter("channel slot is empty".into()))?;
        let entries =
            rekindle_codec::community::channel_record::decode_channel_entries(&value.data)
                .map_err(|e| ChannelError::Adapter(format!("channel slot: {e}")))?;
        let message = entries
            .into_iter()
            .find_map(|entry| match entry {
                ChannelRecordEntry::Message(m)
                    if m.message_id.as_deref() == Some(message_id)
                        && m.sender_pseudonym.eq_ignore_ascii_case(author) =>
                {
                    Some(m)
                }
                _ => None,
            })
            .ok_or_else(|| ChannelError::Adapter(format!("slot holds no message {message_id}")))?;
        self.ingest(ReceivedPost {
            community_id: community_id.to_owned(),
            channel_id: channel_id.to_owned(),
            record_key,
            subkey_index,
            sequence,
            message,
            forwarded_from_author: None,
            attachment_json: None,
        })
        .await
    }

    /// Ingest one received post: open, gate, store, publish.
    ///
    /// # Errors
    /// No identity is unlocked, or the store failed.
    pub async fn ingest(&self, post: ReceivedPost) -> Result<Ingested, ChannelError> {
        let (owner, db) = self.owner_and_db()?;
        let message_id = (post.message.message_id.clone())
            .ok_or_else(|| ChannelError::Adapter("a received post has no message id".into()))?;
        let (key, channel, id) = (owner.clone(), post.channel_id.clone(), message_id.clone());
        let held = db
            .call(move |conn| rows::row_id_by_message_id(conn, &key, &channel, &id))
            .await
            .map_err(stored)?;
        if held.is_some() {
            return Ok(Ingested::Duplicate);
        }
        let community = post.community_id.as_str();
        let Some(body) = ingest::open_body(
            self,
            community,
            &post.channel_id,
            &post.record_key,
            post.subkey_index,
            &post.message,
        ) else {
            return Ok(Ingested::Undecryptable);
        };
        let (blurred, report) = match self.judge(&owner, &db, &post, &body).await? {
            Verdict::Admit => (false, None),
            Verdict::Report(GateReason::AutoMod { rule }) => (false, Some(rule)),
            Verdict::Report(reason) => (false, Some(format!("{reason:?}"))),
            Verdict::Hide(_) => (true, None),
            Verdict::Drop(reason) => {
                tracing::debug!(community, channel = %post.channel_id, ?reason, "post dropped");
                return Ok(Ingested::Dropped(reason));
            }
        };
        let own =
            self.my_pseudonym_impl(community).as_deref() == Some(&post.message.sender_pseudonym);
        let mentions_me =
            !own && rekindle_channel::mentions::message_mentions_me(self, community, &post.message);
        let row = Row {
            message_id: message_id.clone(),
            body: body.clone(),
            blurred,
            own,
            mentions_me,
        };
        self.store_received(owner, &db, post.clone(), row).await?;
        self.ctx().publish_event(SubscriptionEvent::ChannelMessage(
            ChannelMessageEvent::New {
                community: post.community_id.clone(),
                channel: post.channel_id.clone(),
                message_id,
                sender_pseudonym: post.message.sender_pseudonym.clone(),
                sequence: post.sequence,
                timestamp: post.message.timestamp,
                body: Some(body.clone()),
                reply_to_message_id: post.message.reply_to.clone(),
            },
        ));
        if let Err(e) = self
            .gov
            .publish_channel_unread(community, &post.channel_id)
            .await
        {
            tracing::warn!(error = %e, "channel unread count not published");
        }
        if let Some(reason) = report {
            self.report_to_moderators(&post, &reason).await;
        }
        Ok(Ingested::Stored { body, blurred })
    }

    /// An `alert_moderators` match (plan E3.1, D26): a moderator's ingest
    /// writes the post's `ModReport` and raises `AutoModAlert`. The report
    /// id is derived from the post, so every moderator's write is one
    /// queue row.
    async fn report_to_moderators(&self, post: &ReceivedPost, reason: &str) {
        let community = post.community_id.as_str();
        let moderator = self
            .require_channel_permission(community, Some(&post.channel_id), MANAGE_MESSAGES)
            .is_ok();
        let (Some(message_id), Some(channel_id), true) = (
            post.message.message_id.clone(),
            ChannelId::from_hex(&post.channel_id),
            moderator,
        ) else {
            return;
        };
        let mut hasher = blake3::Hasher::new_derive_key("rekindle 2026-10 mod report id");
        hasher.update(community.as_bytes());
        hasher.update(&channel_id.0);
        hasher.update(message_id.as_bytes());
        let mut report_id = [0u8; 16];
        report_id.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
        let held = self
            .governance_state(community)
            .is_some_and(|g| g.mod_reports.contains_key(&report_id));
        if !held {
            if let Err(e) = self
                .write_report(community, report_id, channel_id, &message_id, reason)
                .await
            {
                tracing::warn!(error = %e, community, "moderator report not written");
            }
        }
        self.ctx()
            .publish_event(SubscriptionEvent::System(SystemEvent::AutoModAlert {
                community: post.community_id.clone(),
                channel: post.channel_id.clone(),
                message_id,
                rule_name: reason.to_owned(),
            }));
    }

    async fn write_report(
        &self,
        community: &str,
        report_id: [u8; 16],
        channel_id: ChannelId,
        message_id: &str,
        reason: &str,
    ) -> Result<(), ChannelError> {
        let lamport = self
            .next_governance_lamport(community)
            .map_err(|e| ChannelError::Adapter(e.to_string()))?;
        self.write_governance_entry(
            community,
            GovernanceEntry::ModReport {
                report_id,
                target: ModReportTarget::Message {
                    channel_id,
                    message_id: message_id.to_owned(),
                },
                reason: reason.to_owned(),
                lamport,
            },
        )
        .await
    }

    /// The gate's verdict on `post`, against the merged state, our block
    /// list and the Host's slowmode windows.
    async fn judge(
        &self,
        owner: &str,
        db: &rekindle_db::Db,
        post: &ReceivedPost,
        body: &str,
    ) -> Result<Verdict, ChannelError> {
        let gov = self
            .governance_state(&post.community_id)
            .ok_or_else(|| ChannelError::Adapter("governance state not loaded".into()))?;
        let key = owner.to_owned();
        let blocked = db
            .call(move |conn| rekindle_db::repo::blocked::list(conn, &key))
            .await
            .map_err(stored)?;
        let blocked = BlockList::new(blocked.into_iter().map(|b| b.public_key));
        let item = GateItem {
            community_id: &post.community_id,
            channel_id: &post.channel_id,
            sender_pseudonym: &post.message.sender_pseudonym,
            body,
            sent_at_secs: post.message.timestamp / 1000,
            now_secs: self.ctx().clock.now_secs(),
        };
        let rules = rekindle_channel::compiled_automod_rules(self, &post.community_id, &gov);
        Ok(ingest::gate(
            &item,
            &gov,
            &rules,
            &blocked,
            &mut self.ctx().channel_rate_limits.lock(),
        ))
    }

    async fn store_received(
        &self,
        owner: String,
        db: &rekindle_db::Db,
        post: ReceivedPost,
        row: Row,
    ) -> Result<(), ChannelError> {
        db.call(move |conn| {
            let m = &post.message;
            rows::insert(
                conn,
                &ChannelMessageInsert {
                    reply_to: m.reply_to.as_deref(),
                    owner_key: &owner,
                    channel_id: &post.channel_id,
                    sender_key: &m.sender_pseudonym,
                    body: &row.body,
                    timestamp: i64::try_from(m.timestamp).unwrap_or(i64::MAX),
                    is_read: row.own,
                    mek_generation: Some(i64::try_from(m.mek_generation).unwrap_or(i64::MAX)),
                    message_id: &row.message_id,
                    lamport_ts: m.lamport_ts,
                    automod_blurred: row.blurred,
                    forwarded_from_author: post.forwarded_from_author.as_deref(),
                    flags: m.flags,
                    attachment_json: post.attachment_json.as_deref(),
                    send_state: SendState::Delivered,
                    mentions_me: row.mentions_me,
                },
            )
        })
        .await
        .map_err(stored)
    }
}
