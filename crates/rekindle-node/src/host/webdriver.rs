//! The real-app suite's damaged-post switch (plan E7.14). Compiled only
//! with the `webdriver-test` feature in a debug build, which only
//! `rekindle-desktop`'s own `webdriver-test` feature turns on; never in a
//! release (`lib.rs` compile_error!, `cargo xtask check-webdriver-release`).
//!
//! A real peer cannot send a damaged channel post, so a spec launches the
//! app with `REKINDLE_WDIO_DAMAGE_NEXT_POST=1` (read once at the desktop's
//! startup, `src-tauri/src/webdriver.rs`), which arms this one-shot: the
//! next received post that reaches `open_body` has one ciphertext byte
//! flipped, so the real AEAD fails and the real damaged path runs
//! (`HostChannel::store_undecryptable`). Nothing fakes the result, and no
//! request can arm it.

use std::sync::atomic::Ordering;

use rekindle_codec::community::channel_record::ChannelMessage;

use super::adapters::channel::ingest::ReceivedPost;
use super::Host;
use crate::daemon::dispatch::DaemonContext;

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
