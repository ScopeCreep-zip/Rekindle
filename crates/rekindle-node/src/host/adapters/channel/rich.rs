//! Reactions, threads and polls on the Host (plan E3.1 E3), on both modes:
//! each runs the `rekindle_channel` pipeline over [`HostChannel`] after the
//! permission the request needs. A reaction, poll or vote is a write to our
//! slot of the channel record plus a gossip; a thread is a governance entry
//! and, from its first message, its own record. Every reader takes them
//! from the record (the history merge, `get_poll_results`, the thread page).

use rekindle_channel::deps::ThreadInfoSnapshot;
use rekindle_channel::error::ChannelError;
use rekindle_channel::ChannelMessagingDeps;
use rekindle_types::dto::channel_message::ChannelMessageView;
use rekindle_types::message::SendState;
use rekindle_types::permissions;
use rekindle_types::thread::ThreadInfo;

use super::HostChannel;

/// A thread to open.
pub(crate) struct NewThread<'a> {
    pub channel_id: &'a str,
    pub name: &'a str,
    pub starter_message_id: &'a str,
    pub forum_tag: Option<String>,
    pub auto_archive_seconds: Option<u64>,
}

/// A poll to attach to our message.
pub(crate) struct NewPoll<'a> {
    pub channel_id: &'a str,
    pub message_id: &'a str,
    pub question: &'a str,
    pub answers: Vec<String>,
    pub multi_select: bool,
    pub duration_seconds: Option<u64>,
}

/// Why a thread cannot be unarchived by request: archive state is the
/// governance `ThreadArchived` entry against the thread's last message,
/// so the thread reopens when a member posts to it.
pub(crate) const UNARCHIVE_BY_POST: &str = "archived threads reactivate when a new message is sent";

pub(crate) fn thread_info(snapshot: ThreadInfoSnapshot) -> ThreadInfo {
    ThreadInfo {
        id: snapshot.id,
        channel_id: snapshot.channel_id,
        name: snapshot.name,
        starter_message_id: snapshot.starter_message_id,
        creator_pseudonym: snapshot.creator_pseudonym,
        forum_tag: snapshot.forum_tag,
        created_at: snapshot.created_at,
        archived: snapshot.archived,
        auto_archive_seconds: snapshot.auto_archive_seconds,
        last_message_at: snapshot.last_message_at,
        message_count: snapshot.message_count,
    }
}

pub(crate) fn thread_snapshot(info: ThreadInfo) -> ThreadInfoSnapshot {
    ThreadInfoSnapshot {
        id: info.id,
        channel_id: info.channel_id,
        name: info.name,
        starter_message_id: info.starter_message_id,
        creator_pseudonym: info.creator_pseudonym,
        forum_tag: info.forum_tag,
        created_at: info.created_at,
        archived: info.archived,
        auto_archive_seconds: info.auto_archive_seconds,
        last_message_at: info.last_message_at,
        message_count: info.message_count,
    }
}

fn message_view(view: rekindle_channel::ThreadMessageView) -> ChannelMessageView {
    ChannelMessageView {
        id: 0,
        sender_id: view.sender_pseudonym,
        body: view.body,
        decryption_failed: view.decryption_failed,
        automod_blurred: false,
        timestamp: i64::try_from(view.timestamp_ms).unwrap_or(i64::MAX),
        is_own: view.is_own,
        server_message_id: view.server_message_id,
        reactions: None,
        pinned: None,
        poll: None,
        forwarded_from_author: None,
        attachment: None,
        flags: 0,
        send_state: if view.is_own {
            SendState::Sent
        } else {
            SendState::Delivered
        },
    }
}

impl HostChannel {
    /// The parent channel of `thread_id`.
    fn thread_parent(&self, community_id: &str, thread_id: &str) -> Result<String, ChannelError> {
        self.thread_state(community_id, thread_id)
            .map(|thread| thread.parent_channel_id_hex)
            .ok_or_else(|| ChannelError::InvalidId(format!("thread {thread_id} not found")))
    }

    fn require(&self, community_id: &str, channel_id: &str, bits: u64) -> Result<(), ChannelError> {
        self.require_channel_permission(community_id, Some(channel_id), bits)
    }

    /// Add (`added`) or take off our reaction `emoji` on `message_id`.
    ///
    /// # Errors
    /// We may not send messages here, or the write and the gossip failed.
    pub(crate) async fn react(
        &self,
        community_id: &str,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
        added: bool,
    ) -> Result<(), ChannelError> {
        self.require(community_id, channel_id, permissions::SEND_MESSAGES)?;
        rekindle_channel::persist_reaction(self, community_id, channel_id, message_id, emoji, added)
            .await
    }

    /// Open a thread; its id. The governance merge refuses a thread type
    /// we may not create.
    ///
    /// # Errors
    /// Refused by governance, or the entry or our row failed.
    pub(crate) async fn create_thread(
        &self,
        community_id: &str,
        thread: NewThread<'_>,
    ) -> Result<String, ChannelError> {
        self.require(community_id, thread.channel_id, permissions::VIEW_CHANNELS)?;
        rekindle_channel::create_thread(
            self,
            community_id,
            thread.channel_id,
            thread.name,
            thread.starter_message_id,
            thread.forum_tag,
            thread.auto_archive_seconds,
        )
        .await
    }

    /// Archive `thread_id`.
    ///
    /// # Errors
    /// We may not manage threads, or the entry failed.
    pub(crate) async fn archive_thread(
        &self,
        community_id: &str,
        thread_id: &str,
    ) -> Result<(), ChannelError> {
        let channel = self.thread_parent(community_id, thread_id)?;
        self.require(community_id, &channel, permissions::MANAGE_THREADS)?;
        rekindle_channel::archive_thread(self, community_id, thread_id).await
    }

    /// Reopening a thread is a post to it ([`UNARCHIVE_BY_POST`]).
    ///
    /// # Errors
    /// Always: the thread is unknown, or it reopens by a post.
    pub(crate) fn unarchive_thread(
        &self,
        community_id: &str,
        thread_id: &str,
    ) -> Result<(), ChannelError> {
        let channel = self.thread_parent(community_id, thread_id)?;
        self.require(community_id, &channel, permissions::MANAGE_THREADS)?;
        Err(ChannelError::Adapter(UNARCHIVE_BY_POST.into()))
    }

    /// `channel_id`'s threads (only the open ones when `active_only`).
    ///
    /// # Errors
    /// We may not view the channel, or its governance is not loaded.
    pub(crate) async fn threads(
        &self,
        community_id: &str,
        channel_id: &str,
        active_only: bool,
    ) -> Result<Vec<ThreadInfo>, ChannelError> {
        self.require(community_id, channel_id, permissions::VIEW_CHANNELS)?;
        let threads = if active_only {
            rekindle_channel::list_active_threads(self, community_id, channel_id).await?
        } else {
            rekindle_channel::list_threads(self, community_id, channel_id).await?
        };
        Ok(threads.into_iter().map(thread_info).collect())
    }

    /// A page of `thread_id`'s messages.
    ///
    /// # Errors
    /// We may not view the parent channel, or the record read failed.
    pub(crate) async fn thread_messages(
        &self,
        community_id: &str,
        thread_id: &str,
        limit: u32,
        before_timestamp: Option<u64>,
    ) -> Result<Vec<ChannelMessageView>, ChannelError> {
        let channel = self.thread_parent(community_id, thread_id)?;
        self.require(community_id, &channel, permissions::VIEW_CHANNELS)?;
        let views = rekindle_channel::load_thread_messages(
            self,
            community_id,
            thread_id,
            limit,
            before_timestamp,
        )
        .await?;
        Ok(views.into_iter().map(message_view).collect())
    }

    /// Post `body` to `thread_id`.
    ///
    /// # Errors
    /// We may not send in the parent channel, or the write failed.
    pub(crate) async fn send_thread_message(
        &self,
        community_id: &str,
        thread_id: &str,
        body: &str,
    ) -> Result<(), ChannelError> {
        let channel = self.thread_parent(community_id, thread_id)?;
        self.require(community_id, &channel, permissions::SEND_MESSAGES)?;
        rekindle_channel::send_thread_message(self, community_id, thread_id, body).await
    }

    /// Attach a poll to our message; the poll's id.
    ///
    /// # Errors
    /// We may not send polls, or the write failed.
    pub(crate) async fn create_poll(
        &self,
        community_id: &str,
        poll: NewPoll<'_>,
    ) -> Result<String, ChannelError> {
        self.require(community_id, poll.channel_id, permissions::SEND_POLLS)?;
        rekindle_channel::persist_poll_create(
            self,
            community_id,
            poll.channel_id,
            poll.message_id,
            poll.question,
            poll.answers,
            poll.multi_select,
            poll.duration_seconds,
        )
        .await
    }

    /// Vote `selected_answers` in `poll_id`.
    ///
    /// # Errors
    /// We may not send messages here, the vote is not valid for the poll,
    /// or the write failed.
    pub(crate) async fn vote_poll(
        &self,
        community_id: &str,
        channel_id: &str,
        poll_id: &str,
        selected_answers: Vec<u8>,
    ) -> Result<(), ChannelError> {
        self.require(community_id, channel_id, permissions::SEND_MESSAGES)?;
        rekindle_channel::persist_poll_vote(
            self,
            community_id,
            channel_id,
            poll_id,
            selected_answers,
        )
        .await
    }

    /// Close `poll_id`: its author may, and a member who manages messages.
    ///
    /// # Errors
    /// We are neither, or the write failed.
    pub(crate) async fn close_poll(
        &self,
        community_id: &str,
        channel_id: &str,
        poll_id: &str,
    ) -> Result<(), ChannelError> {
        let moderator = self
            .require(community_id, channel_id, permissions::MANAGE_MESSAGES)
            .is_ok();
        rekindle_channel::persist_poll_close(self, community_id, channel_id, poll_id, moderator)
            .await
    }

    /// `poll_id`'s vote count per answer.
    ///
    /// # Errors
    /// We may not view the channel, or the record read failed.
    pub(crate) async fn poll_results(
        &self,
        community_id: &str,
        channel_id: &str,
        poll_id: &str,
    ) -> Result<Vec<u32>, ChannelError> {
        self.require(community_id, channel_id, permissions::VIEW_CHANNELS)?;
        rekindle_channel::get_poll_results(self, community_id, channel_id, poll_id).await
    }
}
