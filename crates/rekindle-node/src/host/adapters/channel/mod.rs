//! The Host's channel messaging adapter (plan E3.1 E2b): one implementation
//! of `rekindle_channel::ChannelMessagingDeps` over the [`DaemonContext`] —
//! its community store, database, record pool, outbox and governance
//! adapter — on both modes. It replaces the daemon's own send (its
//! `dispatch/channel_message.rs` send over the transport's
//! `operations/channel.rs`), and the channel send of the desktop's
//! `services/channel_adapter`.
//!
//! File split: this module holds the struct, the send entry and the
//! settle task; `deps_impl` is the single trait impl and delegates;
//! `state` the community-store reads and mutations; `dht` the record
//! writes and reads; `persist` the stored rows and the outbox; `events`
//! the window's echo and delivery state.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_channel::error::ChannelError;
use rekindle_channel::ChannelSendResult;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_lifecycle::{ScopeClosed, SessionScope};
use rekindle_outbox::OutboxRow;
use rekindle_records::port::RecordPort;

use crate::daemon::dispatch::DaemonContext;
use crate::host::adapters::governance::HostGovernance;
use crate::host::adapters::outbox::{post_of_row, Reissuer};

pub mod deps_impl;
mod dht;
mod events;
mod persist;
mod state;

pub use dht::delivery;

/// The Host, as the channel pipeline sees it.
#[derive(Clone)]
pub struct HostChannel {
    gov: HostGovernance,
}

impl HostChannel {
    /// The channel adapter over `ctx`.
    pub(crate) fn new(ctx: &DaemonContext) -> Self {
        Self {
            gov: crate::daemon::dispatch::adapter(ctx),
        }
    }

    fn ctx(&self) -> &DaemonContext {
        self.gov.context()
    }

    /// The record pool of the running transport.
    fn records(&self) -> Result<Arc<dyn RecordPort>, ChannelError> {
        let transport = (self.ctx().transport.read().clone())
            .ok_or_else(|| ChannelError::Adapter("transport not started".into()))?;
        transport
            .require_records()
            .map_err(|e| ChannelError::Adapter(format!("record pool: {e}")))
    }

    /// The unlocked identity's key and database.
    fn owner_and_db(&self) -> Result<(String, rekindle_db::Db), ChannelError> {
        let owner = (self.ctx().session.read().as_ref())
            .map(|s| s.identity.public_key_hex.clone())
            .ok_or_else(|| ChannelError::Adapter("no identity unlocked".into()))?;
        let db = (self.ctx().db.current()).map_err(|e| ChannelError::Adapter(e.to_string()))?;
        Ok((owner, db))
    }

    /// Send `body` to `channel_id` of `community_id`, as a reply to the
    /// message at sequence `reply_to` when given: the pipeline seals it,
    /// stores our row, writes our slot of the channel record, gossips the
    /// notification and queues the post on the outbox.
    ///
    /// # Errors
    /// The pipeline's refusal or failure.
    pub(crate) async fn send(
        &self,
        community_id: &str,
        channel_id: &str,
        body: &str,
        reply_to: Option<String>,
    ) -> Result<ChannelSendResult, ChannelError> {
        rekindle_channel::send_channel_message(self, community_id, channel_id, body, reply_to).await
    }
}

/// A send's refusal as the IPC reports it.
pub(crate) fn refusal(error: &ChannelError) -> IpcResponse {
    let code = match error {
        ChannelError::PermissionDenied(_) => 403,
        ChannelError::ChannelNotFound(_) | ChannelError::CommunityNotFound(_) => 404,
        ChannelError::Adapter(message) if message.starts_with("slowmode active") => 429,
        ChannelError::Adapter(message) if message.starts_with("forum channels") => 400,
        _ => 500,
    };
    IpcResponse::error(code, format!("channel send: {error}"))
}

/// The outbox's Retry of a failed channel post: its write is made again
/// under its client id (`rekindle_channel::reissue`).
struct PostReissuer(HostChannel);

#[async_trait]
impl Reissuer for PostReissuer {
    async fn reissue(&self, row: &OutboxRow) -> Result<(), String> {
        let post = post_of_row(row)?;
        rekindle_channel::reissue::reissue_channel_post(&self.0, &post)
            .await
            .map(drop)
            .map_err(|e| e.to_string())
    }
}

/// Follow the session's record pool for settled writes (plan C7.13): a
/// channel write that missed consensus is held and re-pushed by the pool,
/// and when its slot settles the posts waiting on it are delivered, or
/// failed when the network superseded our page. Installs the outbox's
/// Retry of a failed post. Runs on the unlock scope.
///
/// # Errors
/// [`ScopeClosed`] when the unlock already ended.
pub(crate) fn start_settle(
    ctx: &DaemonContext,
    scope: &Arc<SessionScope>,
    pool: &Arc<dyn RecordPort>,
) -> Result<(), ScopeClosed> {
    let channel = HostChannel::new(ctx);
    if let Some(posts) = ctx.outbox.channel_posts() {
        posts.install_reissuer(Arc::new(PostReissuer(channel.clone())));
    }
    let mut settled = pool.settled();
    scope.spawn_with_token("channel delivery settle", |stop| async move {
        loop {
            let event = match stop.run_until_cancelled(settled.recv()).await {
                None | Some(Err(tokio::sync::broadcast::error::RecvError::Closed)) => return,
                Some(Err(tokio::sync::broadcast::error::RecvError::Lagged(missed))) => {
                    // Their slots settle again on their next write.
                    tracing::warn!(missed, "channel delivery settle lagged");
                    continue;
                }
                Some(Ok(event)) => event,
            };
            rekindle_channel::on_write_settled(
                &channel,
                &event.record_key,
                event.subkey,
                event.landed,
            )
            .await;
        }
    })
}
