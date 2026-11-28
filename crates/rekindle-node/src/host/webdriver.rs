//! Real-app suite hooks, available only with `webdriver-test` in debug builds.
//!
//! A real peer cannot send a damaged channel post, so a spec launches the
//! app with `REKINDLE_WDIO_DAMAGE_NEXT_POST=1` (read once at the desktop's
//! startup, `src-tauri/src/webdriver.rs`), which arms this one-shot: the
//! next received post that reaches `open_body` has one ciphertext byte
//! flipped, so the real AEAD fails and the real damaged path runs
//! (`HostChannel::store_undecryptable`). Nothing fakes the result, and no
//! request can arm it.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use parking_lot::Mutex;
use rekindle_governance_runtime::op_log::HeldOp;
use rekindle_governance_runtime::rebuild_governance;
use rekindle_ipc::protocol::IpcResponse;
use tokio::sync::Mutex as AsyncMutex;

use rekindle_codec::community::channel_record::ChannelMessage;

use super::adapters::channel::ingest::ReceivedPost;
use super::Host;
use crate::daemon::dispatch::{adapter, DaemonContext};

impl Host {
    /// Arm the one-shot: the next received channel post is damaged before
    /// it is opened.
    pub fn arm_damage_next_post(&self) {
        self.ctx.damage_next_post.store(true, Ordering::SeqCst);
    }
}

impl DaemonContext {
    /// `post`'s message with its ciphertext's last byte (the AEAD tag's)
    /// flipped, when the one-shot is armed, which disarms it.
    pub(crate) fn damaged_if_armed(&self, post: &ReceivedPost) -> Option<ChannelMessage> {
        if post.message.ciphertext.is_empty()
            || !self.damage_next_post.swap(false, Ordering::SeqCst)
        {
            return None;
        }
        let mut message = post.message.clone();
        if let Some(byte) = message.ciphertext.last_mut() {
            *byte ^= 0x01;
        }
        tracing::warn!(community = %post.community_id, channel = %post.channel_id, "webdriver-test: this received post's ciphertext is damaged");
        Some(message)
    }
}

/// Each community's incoming op store and the hold/release requests share
/// this lock. Local signing writes use their original path.
type IngestHold = Arc<AsyncMutex<Option<VecDeque<HeldOp>>>>;

#[derive(Debug, Clone, Default)]
pub(crate) struct GovernanceIngestHolds(Arc<Mutex<BTreeMap<String, IngestHold>>>);

impl GovernanceIngestHolds {
    pub(crate) fn for_community(&self, community: &str) -> IngestHold {
        self.0
            .lock()
            .entry(community.to_owned())
            .or_default()
            .clone()
    }

    pub(crate) fn remove(&self, community: &str) {
        self.0.lock().remove(community);
    }

    pub(crate) fn clear(&self) {
        self.0.lock().clear();
    }
}

pub(crate) async fn hold_governance_ops(ctx: &DaemonContext, community: &str) -> IpcResponse {
    let membership = match ctx.resolve_community(community) {
        Ok(membership) => membership,
        Err(refusal) => return refusal,
    };
    let hold = ctx
        .communities
        .governance_ingest_holds
        .for_community(&membership.anchor);
    hold.lock().await.get_or_insert_with(VecDeque::new);
    IpcResponse::ok(&serde_json::Value::Null)
}

pub(crate) async fn release_governance_ops(ctx: &DaemonContext, community: &str) -> IpcResponse {
    let membership = match ctx.resolve_community(community) {
        Ok(membership) => membership,
        Err(refusal) => return refusal,
    };
    let community = &membership.anchor;
    let hold = ctx
        .communities
        .governance_ingest_holds
        .for_community(community);
    let gov = adapter(ctx);
    let count = {
        let mut queue = hold.lock().await;
        let Some(queued) = queue.as_mut() else {
            return IpcResponse::error(409, "governance ingest is not held for this community");
        };
        let count = queued.len();
        while let Some(op) = queued.front() {
            if let Err(error) = gov.store_governance_op_impl(community, op, false).await {
                return IpcResponse::error(500, error.to_string());
            }
            queued.pop_front();
        }
        *queue = None;
        count
    };
    rebuild_governance(&gov, community).await;
    IpcResponse::ok(&serde_json::json!({ "released": count }))
}
