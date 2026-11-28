//! The one ingest path for received channel posts (plan E3.1 F3, D26), on
//! both modes: the outbox read and the slot-record reads hand a post here. It opens the body, runs the
//! `rekindle_channel::ingest` gate, stores the row (with `mentions_me`, and
//! read when we wrote it from another device), publishes
//! `ChannelMessageEvent::New` and the channel's `UnreadChanged`.

use rekindle_channel::error::ChannelError;
use rekindle_channel::ingest::{self, BlockList, GateItem, GateReason, SlotPosition, Verdict};
use std::sync::Arc;

use rekindle_channel::{AutoModCompiledCache, ChannelMessagingDeps};
use rekindle_db::repo::messages::channel::{self as rows, ChannelMessageInsert};
use rekindle_governance::state::GovernanceState;
use rekindle_types::channel_keys::KeyEpoch;
use rekindle_types::dto::channel_message::UndecryptableReason;
use rekindle_types::id::{ChannelId, PseudonymKey};
use rekindle_types::message::SendState;
use rekindle_types::permissions::VIEW_CHANNELS;
use rekindle_types::subscription_events::{ChannelMessageEvent, SubscriptionEvent};

pub use super::received::{ReceivedAt, ReceivedPost};
use super::HostChannel;

fn stored(e: impl std::fmt::Display) -> ChannelError {
    ChannelError::Adapter(format!("store: {e}"))
}

/// The columns of a received row the ingest decides.
struct Row {
    message_id: String,
    body: String,
    blurred: bool,
    own: bool,
    mentions_me: bool,
    /// Stored without its body, and why (`store_undecryptable`).
    undecryptable: Option<UndecryptableReason>,
}

/// One opened post as the gate judges it, from any receive path.
pub(crate) struct GatedPost<'a> {
    pub community_id: &'a str,
    pub channel_id: &'a str,
    pub sender_pseudonym: &'a str,
    pub body: &'a str,
    /// The post's own timestamp, in milliseconds.
    pub timestamp_ms: u64,
    /// The author's nearest held post at or before it, in seconds
    /// ([`HostChannel::stored_before_secs`], and in a batch
    /// `ingest::BatchWindows`).
    pub held_before_secs: Option<u64>,
    /// Read from history, not received live ([`GateItem::backfill`]).
    pub backfill: bool,
    /// Where a history read found it ([`GateItem::slot`]).
    pub slot: Option<SlotPosition<'a>>,
}

/// The community state a batch of posts is judged against
/// ([`HostChannel::gate_inputs`]).
pub(crate) struct GateInputs {
    gov: GovernanceState,
    rules: Arc<AutoModCompiledCache>,
    blocked: BlockList,
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
    /// We were removed from the community: nothing of it is opened or
    /// stored, whichever path read it (plan E3.2 S9, R3-1).
    Removed,
    /// A moderator's merged `AdminDelete` names it (plan E3.3 M4): it is
    /// stored removed, never opened.
    RemovedByModerator,
    /// We may not view its channel (plan E3.2 SEC): nothing of it is
    /// opened, stored or published. The outbox cursor passes it; the merge
    /// that opens the channel to us re-reads every outbox
    /// (`governance/view_gain.rs`, the D16 re-read), so it is read then.
    Withheld,
}

impl HostChannel {
    /// Ingest one received post: gate its author, open, gate, store,
    /// publish.
    ///
    /// # Errors
    /// No identity is unlocked, or the store failed.
    pub async fn ingest(&self, post: ReceivedPost) -> Result<Ingested, ChannelError> {
        self.ingest_as(post, false, false).await
    }

    pub(super) async fn ingest_authorized(
        &self,
        post: ReceivedPost,
        backfill: bool,
    ) -> Result<Ingested, ChannelError> {
        self.ingest_as(post, backfill, true).await
    }

    /// [`ingest`](Self::ingest), with `backfill` when the post was read from
    /// history rather than received live ([`GateItem::backfill`]). The
    /// author checks ([`ingest::author_refusal`]) run before the body is
    /// opened, so a refused author's post is never stored or reported
    /// undecryptable, whichever path read it (A/B finding R2-4).
    ///
    /// # Errors
    /// No identity is unlocked, or the store failed.
    async fn ingest_as(
        &self,
        post: ReceivedPost,
        backfill: bool,
        causal_timeout_checked: bool,
    ) -> Result<Ingested, ChannelError> {
        if self
            .ctx()
            .communities
            .own_removal(&post.community_id)
            .is_some()
        {
            return Ok(Ingested::Removed);
        }
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
        if let Some(by) = self.removed_by_moderator(&post, &message_id) {
            return self.store_removed(owner, &db, post, message_id, by).await;
        }
        let community = post.community_id.as_str();
        let mut inputs = self.gate_inputs(&owner, &db, community).await?;
        if causal_timeout_checked {
            // This author's timeout was evaluated at the signed cut. D26
            // still uses the current local block list and departure boundary.
            let author = PseudonymKey::from_hex_lossy(&post.message.sender_pseudonym);
            inputs.gov.timeouts.remove(&author);
        }
        let author = GatedPost {
            community_id: community,
            channel_id: &post.channel_id,
            sender_pseudonym: &post.message.sender_pseudonym,
            body: "",
            timestamp_ms: post.message.timestamp,
            held_before_secs: None,
            // An outbox entry placed at or below a departure's head is
            // judged at that causal position, whichever trigger read it.
            backfill: backfill || post.departure_position().is_some(),
            // A live post, or the desktop's catch-up read, which does not
            // carry the slot seq, keeps none of a removed author's posts;
            // an outbox read carries the head the chain verified (A/B
            // finding R3-8, plan E3.3).
            slot: post.departure_position(),
        };
        if let Some(reason) = self.author_refused(&inputs, &author) {
            tracing::debug!(community, channel = %post.channel_id, ?reason, "post dropped");
            return Ok(Ingested::Dropped(reason));
        }
        if !self.we_may_view(&inputs.gov, community, &post.channel_id) {
            tracing::info!(community, channel = %post.channel_id, "channel post withheld: we may not view its channel");
            return Ok(Ingested::Withheld);
        }
        // The real-app suite's one-shot damages this post's ciphertext, so
        // the AEAD below fails for real (`host::webdriver`).
        #[cfg(all(feature = "webdriver-test", debug_assertions))]
        let damaged = self.ctx().damaged_if_armed(&post);
        #[cfg(all(feature = "webdriver-test", debug_assertions))]
        let message = damaged.as_ref().unwrap_or(&post.message);
        #[cfg(not(all(feature = "webdriver-test", debug_assertions)))]
        let message = &post.message;
        let Some(body) =
            ingest::open_body(self, community, &post.channel_id, post.body_at(), message)
        else {
            return Ok(Ingested::Undecryptable);
        };
        let verdict = (self.judge_with(&inputs, &owner, &db, &post, &body, backfill)).await?;
        let (blurred, report) = match verdict {
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
        let report_id = message_id.clone();
        let row = Row {
            message_id: message_id.clone(),
            body: body.clone(),
            blurred,
            own,
            mentions_me,
            undecryptable: None,
        };
        self.store_received(owner, &db, post.clone(), row).await?;
        let author = PseudonymKey::from_hex_lossy(&post.message.sender_pseudonym);
        self.release_held(&post.community_id, &author, &message_id)
            .await;
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
                forwarded_from_author: post.forwarded_from_author.clone(),
                forwarded_from_name: self
                    .forward_name(&post.community_id, post.forwarded_from_author.as_deref()),
                automod_blurred: blurred,
                undecryptable: None,
            },
        ));
        // The typist's message ends their typing here (plan E3.4 S9).
        if !own {
            rekindle_transport::operations::typing::stopped(
                &crate::host::adapters::friends::HostFriends::of(self.ctx()),
                &rekindle_types::subscription_events::TypingContext::Channel {
                    community: post.community_id.clone(),
                    channel: post.channel_id.clone(),
                },
                &post.message.sender_pseudonym,
            );
        }
        if let Err(e) = self
            .gov
            .publish_channel_unread(community, &post.channel_id)
            .await
        {
            tracing::warn!(error = %e, "channel unread count not published");
        }
        if let Some(reason) = report {
            self.report_to_moderators(community, &post.channel_id, &report_id, &reason)
                .await;
        }
        Ok(Ingested::Stored { body, blurred })
    }

    /// The gate's verdict on `post`, against `inputs` (the merged state,
    /// our block list and rules, read once by the caller) and the author's
    /// nearest stored post at or before it.
    pub(crate) async fn judge_with(
        &self,
        inputs: &GateInputs,
        owner: &str,
        db: &rekindle_db::Db,
        post: &ReceivedPost,
        body: &str,
        backfill: bool,
    ) -> Result<Verdict, ChannelError> {
        let held_before_secs = self
            .stored_before_secs(
                owner,
                db,
                &post.channel_id,
                &post.message.sender_pseudonym,
                post.message.timestamp,
                post.message.message_id.as_deref(),
            )
            .await?;
        Ok(self.verdict(
            inputs,
            &GatedPost {
                community_id: &post.community_id,
                channel_id: &post.channel_id,
                sender_pseudonym: &post.message.sender_pseudonym,
                body,
                timestamp_ms: post.message.timestamp,
                held_before_secs,
                backfill: backfill || post.departure_position().is_some(),
                slot: post.departure_position(),
            },
        ))
    }

    /// The second of `sender`'s latest stored post in `channel_id` at or
    /// before the second of `timestamp_ms`, other than the post
    /// `message_id` itself: the post the slowmode judges against (plan
    /// E3.1 FX2). Stored rows are exactly the admitted and hidden posts,
    /// so the answer is the same in every session.
    ///
    /// # Errors
    /// The read failed.
    pub(crate) async fn stored_before_secs(
        &self,
        owner: &str,
        db: &rekindle_db::Db,
        channel_id: &str,
        sender: &str,
        timestamp_ms: u64,
        message_id: Option<&str>,
    ) -> Result<Option<u64>, ChannelError> {
        let last_ms_of_second = (timestamp_ms / 1000)
            .saturating_mul(1000)
            .saturating_add(999);
        let bound = i64::try_from(last_ms_of_second).unwrap_or(i64::MAX);
        let (key, channel, sender) = (owner.to_owned(), channel_id.to_owned(), sender.to_owned());
        let id = message_id.map(str::to_owned);
        let found = db
            .call(move |conn| {
                rows::latest_by_sender_at_or_before(
                    conn,
                    &key,
                    &channel,
                    &sender,
                    bound,
                    id.as_deref(),
                )
            })
            .await
            .map_err(stored)?;
        Ok(found.map(|ms| u64::try_from(ms).unwrap_or(0) / 1000))
    }

    /// What the gate judges a community's posts against: its merged state,
    /// its compiled automod rules and our block list, read once for every
    /// post of one batch (a live post, or a backfilled page).
    ///
    /// # Errors
    /// The community's governance state is not loaded, or the block list
    /// read failed.
    pub(crate) async fn gate_inputs(
        &self,
        owner: &str,
        db: &rekindle_db::Db,
        community_id: &str,
    ) -> Result<GateInputs, ChannelError> {
        let gov = self
            .governance_state(community_id)
            .ok_or_else(|| ChannelError::Adapter("governance state not loaded".into()))?;
        let (key, community) = (owner.to_owned(), community_id.to_owned());
        let blocked = db
            .call(move |conn| rekindle_db::repo::community_blocks::list(conn, &key, &community))
            .await
            .map_err(stored)?;
        let rules = rekindle_channel::compiled_automod_rules(self, community_id, &gov);
        Ok(GateInputs {
            gov,
            rules,
            blocked: BlockList::new(blocked.into_iter().map(|b| b.pseudonym)),
        })
    }

    /// The gate's verdict on one post, against `inputs` and its
    /// `held_before_secs`: the one judgement every received post passes,
    /// whatever path brought it in (live ingest, notified fetch or the
    /// history backfill).
    pub(crate) fn verdict(&self, inputs: &GateInputs, post: &GatedPost<'_>) -> Verdict {
        ingest::gate(
            &self.gate_item(post),
            &inputs.gov,
            &inputs.rules,
            &inputs.blocked,
        )
    }

    /// The gate's author checks alone ([`ingest::author_refusal`]): run on
    /// a post before its body is opened, and on one no held key opens.
    pub(crate) fn author_refused(
        &self,
        inputs: &GateInputs,
        post: &GatedPost<'_>,
    ) -> Option<GateReason> {
        ingest::author_refusal(&self.gate_item(post), &inputs.gov, &inputs.blocked)
    }

    /// Whether we hold `VIEW_CHANNELS` on `channel_id` in the merged
    /// state (plan E3.2 SEC; `compute_permissions` with the channel's
    /// overwrites). This is a reader-side gate: every channel's posts are
    /// sealed under the community key we hold, so until E3.5's per-channel
    /// keys (D11) make it cryptographic, this check is what keeps a channel
    /// an overwrite closes to us unread. An own key not known yet is judged
    /// as no member's: `@everyone` and its overwrites alone.
    pub(super) fn we_may_view(
        &self,
        state: &GovernanceState,
        community_id: &str,
        channel_id: &str,
    ) -> bool {
        let me =
            PseudonymKey::from_hex_lossy(&self.my_pseudonym_impl(community_id).unwrap_or_default());
        let Some(channel) = ChannelId::from_hex(channel_id) else {
            return false;
        };
        let now_secs = self.ctx().clock.now_secs();
        let perms = rekindle_governance::permissions::compute_permissions(
            &me,
            Some(&channel),
            state,
            now_secs,
        );
        perms & VIEW_CHANNELS != 0
    }

    fn gate_item<'a>(&self, post: &GatedPost<'a>) -> GateItem<'a> {
        GateItem {
            community_id: post.community_id,
            channel_id: post.channel_id,
            sender_pseudonym: post.sender_pseudonym,
            body: post.body,
            sent_at_secs: post.timestamp_ms / 1000,
            held_before_secs: post.held_before_secs,
            now_secs: self.ctx().clock.now_secs(),
            backfill: post.backfill,
            slot: post.slot,
        }
    }

    /// Store a post a moderator removed before it reached us (plan E3.3
    /// M4, delete-wins): an empty row tombstoned as removed by `by`.
    async fn store_removed(
        &self,
        owner: String,
        db: &rekindle_db::Db,
        post: ReceivedPost,
        message_id: String,
        by: PseudonymKey,
    ) -> Result<Ingested, ChannelError> {
        let row = Row {
            message_id: message_id.clone(),
            body: String::new(),
            blurred: false,
            own: false,
            mentions_me: false,
            undecryptable: None,
        };
        self.store_received(owner, db, post.clone(), row).await?;
        (self.gov)
            .store_admin_delete(
                &post.community_id,
                &post.channel_id,
                &message_id,
                &by,
                &post.message.sender_pseudonym,
            )
            .await
            .map_err(|e| ChannelError::Adapter(e.to_string()))?;
        Ok(Ingested::RemovedByModerator)
    }

    /// Whether we hold the text key of `channel_id` at `generation`.
    fn holds_text_key(&self, community_id: &str, channel_id: &str, generation: u64) -> bool {
        let Some(channel) = ChannelId::from_hex(channel_id) else {
            return false;
        };
        let keys = self.keys();
        let scope = keys.scope_for_text(community_id, channel);
        keys.key(community_id, scope, KeyEpoch(generation))
            .is_some()
    }

    /// A post its author's signed outbox entry carries, whose body did not
    /// open ([`Ingested::Undecryptable`]): when we hold the key of the
    /// generation it names, the ciphertext itself is damaged, so it is
    /// stored without its body, visibly unreadable, and published so; never
    /// skipped past invisibly (plan KP). `false` when we do not hold that
    /// key: the caller holds the entry for it or leaves it as history from
    /// before our keys.
    ///
    /// # Errors
    /// No identity is unlocked, or the store failed.
    pub(crate) async fn store_undecryptable(
        &self,
        post: ReceivedPost,
    ) -> Result<bool, ChannelError> {
        let generation = post.message.mek_generation;
        if !self.holds_text_key(&post.community_id, &post.channel_id, generation) {
            return Ok(false);
        }
        let (owner, db) = self.owner_and_db()?;
        let message_id = (post.message.message_id.clone())
            .ok_or_else(|| ChannelError::Adapter("a received post has no message id".into()))?;
        let reason = UndecryptableReason::Damaged;
        let row = Row {
            message_id: message_id.clone(),
            body: String::new(),
            blurred: false,
            own: false,
            mentions_me: false,
            undecryptable: Some(reason),
        };
        self.store_received(owner, &db, post.clone(), row).await?;
        tracing::warn!(community = %post.community_id, channel = %post.channel_id, generation, "channel post stored unreadable: its body does not open under the key it names");
        // The forward's original author, as the stored row holds it and
        // history serves it.
        let forwarded_from_name =
            self.forward_name(&post.community_id, post.forwarded_from_author.as_deref());
        self.ctx().publish_event(SubscriptionEvent::ChannelMessage(
            ChannelMessageEvent::New {
                community: post.community_id,
                channel: post.channel_id,
                message_id,
                sender_pseudonym: post.message.sender_pseudonym,
                sequence: post.sequence,
                timestamp: post.message.timestamp,
                body: None,
                reply_to_message_id: post.message.reply_to,
                forwarded_from_author: post.forwarded_from_author,
                forwarded_from_name,
                automod_blurred: false,
                undecryptable: Some(reason.into()),
            },
        ));
        Ok(true)
    }

    async fn store_received(
        &self,
        owner: String,
        db: &rekindle_db::Db,
        post: ReceivedPost,
        row: Row,
    ) -> Result<(), ChannelError> {
        // The post's embedded offer is the attachment's record, the same
        // record the uploader stored; a later download fills its path.
        let attachment_json = (post.message.attachment.as_ref())
            .map(|offer| {
                serde_json::to_string(&rekindle_files::AttachmentRecordJson::from_offer(
                    offer, None,
                ))
            })
            .transpose()
            .map_err(stored)?;
        db.call(move |conn| {
            let m = &post.message;
            let insert = rows::insert(
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
                    attachment_json: attachment_json.as_deref(),
                    send_state: SendState::Delivered,
                    mentions_me: row.mentions_me,
                },
            );
            match (insert, row.undecryptable) {
                (Ok(()), Some(reason)) => rows::mark_undecryptable(
                    conn,
                    &owner,
                    &post.channel_id,
                    &row.message_id,
                    reason.as_str(),
                ),
                (insert, _) => insert,
            }
        })
        .await
        .map_err(stored)
    }
}
