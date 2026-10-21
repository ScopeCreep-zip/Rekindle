//! The channel pipeline's posts on the Host's outbox `channel_post` stream
//! (plan E2.5): the Host owns the rows and their state; this only converts
//! the Host's outbox errors to the pipeline's, and forwards the outbox's
//! Retry of a failed post to the pipeline's rewrite.

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_channel::deps::ChannelPost;
use rekindle_channel::error::ChannelError;
use rekindle_node::host::adapters::outbox::{post_of_row, ChannelPosts, Reissuer};
use rekindle_outbox::OutboxRow;

use super::ChannelAdapter;
use crate::state::AppState;

/// The outbox's Retry of a failed channel post, forwarded to
/// `rekindle_channel::reissue::reissue_channel_post`.
pub struct PostReissuer(pub Arc<AppState>);

#[async_trait]
impl Reissuer for PostReissuer {
    async fn reissue(&self, row: &OutboxRow) -> Result<(), String> {
        let post = post_of_row(row)?;
        let adapter = crate::services::build_adapter(&self.0, ChannelAdapter::new)?;
        rekindle_channel::reissue::reissue_channel_post(&adapter, &post)
            .await
            .map(drop)
            .map_err(|e| e.to_string())
    }
}

impl ChannelAdapter {
    fn channel_posts(&self) -> Result<ChannelPosts, ChannelError> {
        (self.state.host.current())
            .and_then(|host| host.channel_posts())
            .ok_or_else(|| ChannelError::Adapter("no unlocked outbox".into()))
    }

    pub(super) async fn queue_post_impl(&self, post: &ChannelPost) -> Result<(), ChannelError> {
        (self.channel_posts()?.queue(post).await).map_err(|e| ChannelError::Adapter(e.to_string()))
    }

    pub(super) async fn settle_posts_impl(
        &self,
        record_key: &str,
        subkey: u32,
        landed: bool,
    ) -> Vec<ChannelPost> {
        let settled = match self.channel_posts() {
            Ok(posts) => posts.settle(record_key, subkey, landed).await,
            Err(error) => {
                tracing::warn!(%error, "channel posts not settled");
                return Vec::new();
            }
        };
        match settled {
            Ok(settled) => settled,
            Err(error) => {
                tracing::warn!(%error, "settling the channel posts failed");
                Vec::new()
            }
        }
    }

    pub(super) async fn fail_post_impl(&self, message: &str) {
        let failed = match self.channel_posts() {
            Ok(posts) => posts.fail(message).await.map_err(|e| e.to_string()),
            Err(error) => Err(error.to_string()),
        };
        if let Err(error) = failed {
            tracing::warn!(%error, message, "failing the channel post failed");
        }
    }
}
