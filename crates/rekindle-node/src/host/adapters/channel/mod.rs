//! The Host's channel messaging adapter (plan E3.1 E2b): one implementation
//! of `rekindle_channel::ChannelMessagingDeps` over the [`DaemonContext`] —
//! its community store, database, record pool, outbox and governance
//! adapter — on both modes. It replaces the daemon's own send (its
//! `dispatch/channel_message.rs` send over the transport's
//! `operations/channel.rs`), and the desktop's former channel adapter.
//!
//! File split: this module holds the struct, the send entry and the
//! settle task; `deps_impl` is the single trait impl and delegates;
//! `state` the community-store reads and mutations; `dht` the record
//! writes and reads; `persist` the stored rows and the outbox; `ingest`
//! the received posts; `blocks` the community blocks its gate reads; `events` the echo and delivery state on the Host bus; `history` the reads.

use std::sync::Arc;

use rekindle_channel::error::ChannelError;
use rekindle_channel::ChannelSendResult;
use rekindle_ipc::protocol::IpcResponse;
use rekindle_lifecycle::{ScopeClosed, SessionScope};
use rekindle_records::port::RecordPort;

use crate::daemon::dispatch::DaemonContext;
use crate::host::adapters::governance::HostGovernance;

mod blocks;
pub(crate) mod changes;
pub mod deps_impl;
mod dht;
mod events;
pub(crate) mod held_changes;
pub(crate) mod history;
pub mod ingest;
pub(crate) mod not_delivered;
mod outbox_entries;
mod outbox_read;
pub(crate) mod outbox_send;
mod persist;
mod received;
mod record_change;
pub(crate) mod rich;
pub mod stage_sound;
mod state;
mod thread_receive;
mod thread_refusal;

pub use dht::delivery;

/// The Host, as the channel pipeline sees it.
#[derive(Clone)]
pub struct HostChannel {
    gov: HostGovernance,
}

impl crate::host::Host {
    /// The channel adapter over this Host.
    #[must_use]
    pub fn channel(&self) -> HostChannel {
        HostChannel::new(&self.ctx)
    }
}

impl HostChannel {
    /// The channel adapter over `gov`'s Host.
    pub(crate) fn of_governance(gov: &HostGovernance) -> Self {
        Self { gov: gov.clone() }
    }

    /// The channel adapter over `ctx`.
    pub(crate) fn new(ctx: &DaemonContext) -> Self {
        Self {
            gov: crate::daemon::dispatch::adapter(ctx),
        }
    }

    /// The Host's chunk caches, for the expression store.
    fn files(&self) -> crate::host::adapters::files::HostFiles {
        crate::host::adapters::files::HostFiles::new(self.gov.clone())
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
        self.refuse_removed(community_id)?;
        rekindle_channel::send_channel_message(self, community_id, channel_id, body, reply_to).await
    }

    /// Refuse a write to a community we were removed from: it is read-only
    /// history (plan E3.2 S9 own removal). Reads stay allowed.
    ///
    /// # Errors
    /// `PermissionDenied` once the merge found our removal.
    pub(crate) fn refuse_removed(&self, community_id: &str) -> Result<(), ChannelError> {
        match self.ctx().communities.own_removal(community_id) {
            Some(removal) => Err(ChannelError::Removed {
                banned: removal.banned,
            }),
            None => Ok(()),
        }
    }

    /// Forward our stored copy of a message into another channel (plan
    /// E3.1 E2d): the pipeline seals its body under the destination's key,
    /// stores our row, writes our slot as a `ChannelForward` and gossips it.
    ///
    /// # Errors
    /// The pipeline's refusal or failure (the source is not stored here).
    pub(crate) async fn forward(
        &self,
        params: rekindle_channel::ForwardChannelMessageParams<'_>,
    ) -> Result<ChannelSendResult, ChannelError> {
        self.refuse_removed(params.dest_community)?;
        rekindle_channel::forward_channel_message(self, params).await
    }
}

/// A send's refusal as the IPC reports it.
pub(crate) fn refusal(error: &ChannelError) -> IpcResponse {
    if let Some(removed) = removed_refusal(error, "channel send") {
        return removed;
    }
    let code = match error {
        ChannelError::PermissionDenied(_) => 403,
        ChannelError::ChannelNotFound(_) | ChannelError::CommunityNotFound(_) => 404,
        ChannelError::Adapter(message) if message.starts_with("slowmode active") => 429,
        ChannelError::Adapter(message) if message.starts_with("forum channels") => 400,
        _ => 500,
    };
    IpcResponse::error(code, format!("channel send: {error}"))
}

/// What a member removed from a community can do (plan E3.2 S9 R3-4,
/// Element's "You were removed … Forget this room"): not unlock, which the
/// CLI suggests for a bare 403.
pub(crate) const REMOVED_REMEDIATION: &str =
    "you can rejoin with a new invite, or forget the community (rekindle community forget)";

/// `error` as a 403 naming what to do, when it is our own removal.
pub(crate) fn removed_refusal(error: &ChannelError, what: &str) -> Option<IpcResponse> {
    matches!(error, ChannelError::Removed { .. }).then(|| {
        IpcResponse::error_with_remediation(403, format!("{what}: {error}"), REMOVED_REMEDIATION)
    })
}

/// What a post that waited past its deadline for its channel's key shows
/// (plan E3.2 S7l, F9-1); Retry and Delete stay on it.
pub(crate) fn key_did_not_arrive_text() -> String {
    format!("Not sent: {}", rekindle_outbox::KEY_DID_NOT_ARRIVE)
}

/// Tell the windows of each post that waited past its deadline for its
/// channel's key (the outbox engine fails it with
/// [`rekindle_outbox::KEY_DID_NOT_ARRIVE`]), with the reason they show.
/// Runs on the unlock scope.
///
/// # Errors
/// [`ScopeClosed`] when the unlock already ended.
pub(crate) fn start_key_deadline(
    ctx: &DaemonContext,
    scope: &Arc<SessionScope>,
) -> Result<(), ScopeClosed> {
    let Some(posts) = ctx.outbox.channel_posts() else {
        return Ok(());
    };
    let channel = HostChannel::new(ctx);
    let mut changes = posts.changes();
    scope.spawn_with_token("channel post key deadline", |stop| async move {
        loop {
            let change = match stop.run_until_cancelled(changes.recv()).await {
                None | Some(Err(tokio::sync::broadcast::error::RecvError::Closed)) => return,
                Some(Err(tokio::sync::broadcast::error::RecvError::Lagged(missed))) => {
                    tracing::warn!(missed, "channel post states missed; the rows hold them");
                    continue;
                }
                Some(Ok(change)) => change,
            };
            let expired = change.kind == rekindle_outbox::StreamKind::ChannelPost
                && change.state == rekindle_types::message::SendState::Failed
                && change.wedge_reason.as_deref() == Some(rekindle_outbox::KEY_DID_NOT_ARRIVE);
            if !expired {
                continue;
            }
            match posts.post(&change.client_id).await {
                Ok(Some(post)) => {
                    // Stored with the row, so a reload still says why.
                    channel
                        .store_failure_reason(&post.message, &key_did_not_arrive_text())
                        .await;
                    channel.send_state_impl(
                        &post.community,
                        &post.channel,
                        &post.message,
                        rekindle_types::message::SendState::Failed,
                        Some(key_did_not_arrive_text()),
                    );
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "expired channel post not read"),
            }
        }
    })
}

/// A key of `community_id` was installed (plan E3.2 S7l, F9-1): its posts
/// waiting for it are released to their writes, on the unlock scope. The
/// install is the trigger; nothing polls.
pub(crate) fn release_waiting_posts(gov: &HostGovernance, community_id: &str) {
    if gov.context().outbox.channel_posts().is_none() {
        return;
    }
    let channel = HostChannel { gov: gov.clone() };
    let community = community_id.to_owned();
    let releases = gov.context().outbox.channel_release_lock();
    rekindle_governance_runtime::deps::GovernanceRuntimeDeps::scope(gov).spawn_or_drop(
        "channel posts released",
        async move {
            // One release at a time: two would race to claim a channel's
            // posts and could finish their writes out of order.
            let _one_at_a_time = releases.lock().await;
            rekindle_channel::awaiting_key::release_waiting_posts(&channel, &community).await;
        },
    );
}

/// We left `community_id`, or were removed from it: its posts still
/// waiting for the key are discarded, never written (plan E3.2 S7l, as
/// `mek_transfer`'s `DEPARTED`).
pub(crate) fn discard_waiting_posts(gov: &HostGovernance, community_id: &str) {
    let Some(posts) = gov.context().outbox.channel_posts() else {
        return;
    };
    let community = community_id.to_owned();
    rekindle_governance_runtime::deps::GovernanceRuntimeDeps::scope(gov).spawn_or_drop(
        "waiting channel posts discarded",
        async move {
            match posts.discard_community(&community).await {
                Ok(discarded) if !discarded.is_empty() => {
                    tracing::info!(community = %community, count = discarded.len(), "waiting channel posts discarded: departed");
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(community = %community, %error, "waiting channel posts not discarded");
                }
            }
        },
    );
}
