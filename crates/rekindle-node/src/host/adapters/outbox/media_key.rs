//! The `media_key` stream (plan E2.5, C7.22): each call media key we send
//! is a row of the stream of its (channel session, recipient), its payload
//! the key already sealed to the recipient and signed (never the key
//! itself). The attempt goes through the [`MediaKeyCarrier`] the GUI
//! installs, which holds the voice transport and keyring: it settles the
//! row unsent when the key is no longer current or the recipient left,
//! and otherwise calls the recipient's roster route, whose acknowledgement
//! settles the row `delivered`. The session's end discards its rows; the
//! horizon is only an upper bound. A `media_key` row is never the user's
//! to retry (`rekindle_outbox::OutboxError::NotRetryable`).

use std::sync::Arc;

use async_trait::async_trait;
use rekindle_outbox::{
    AttemptOutcome, Deadline, Entry, Outbox, OutboxError, OutboxRow, StreamKind,
};

use super::{HostOutbox, OutboxSlot, StreamSender};
use crate::host::runtime::Host;

/// The reason a key no longer current, or whose recipient left, settles
/// with.
pub const SUPERSEDED: &str = "superseded";
/// The reason a session's rows are discarded with when it ends.
pub const SESSION_ENDED: &str = "session ended";

/// How one delivery attempt of a sealed media key went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKeyDelivery {
    /// The recipient acknowledged exactly this key.
    Acknowledged,
    /// No acknowledgement (the recipient is unreachable now); try again.
    Unacknowledged,
    /// Not sent: a later rotation replaced the key, or the recipient left.
    Superseded,
}

/// What the voice side gives the Host's `media_key` sender.
#[async_trait]
pub trait MediaKeyCarrier: Send + Sync {
    /// Deliver the sealed key `payload` once, if it is still current.
    async fn deliver(&self, payload: &[u8]) -> MediaKeyDelivery;
}

struct MediaKeySender(Arc<dyn MediaKeyCarrier>);

#[async_trait]
impl StreamSender for MediaKeySender {
    async fn send(&self, row: &OutboxRow) -> AttemptOutcome {
        match self.0.deliver(&row.payload).await {
            MediaKeyDelivery::Acknowledged => AttemptOutcome::Receipted,
            MediaKeyDelivery::Unacknowledged => AttemptOutcome::Deferred,
            MediaKeyDelivery::Superseded => AttemptOutcome::Superseded(SUPERSEDED.to_owned()),
        }
    }
}

fn scope(session: &str, recipient: &str) -> String {
    format!("{session}/{recipient}")
}

/// The unlocked identity's `media_key` streams.
pub struct MediaKeys(Arc<Outbox<HostOutbox>>);

impl MediaKeys {
    /// Queue `sealed`, a key sealed to `recipient`, on the stream of
    /// (`session`, `recipient`) as `client_id`.
    ///
    /// # Errors
    /// The storage failed, or `client_id` is already queued.
    pub async fn queue(
        &self,
        session: &str,
        recipient: &str,
        client_id: &str,
        sealed: Vec<u8>,
    ) -> Result<(), OutboxError> {
        let entry = Entry {
            kind: StreamKind::MediaKey,
            scope: scope(session, recipient),
            client_id: client_id.to_owned(),
            payload: sealed,
            deadline: Deadline::Horizon,
        };
        self.0.enqueue(entry, None).await.map(drop)
    }

    /// `session` ended: discard every key still queued for any of its
    /// recipients; how many.
    ///
    /// # Errors
    /// The storage failed.
    pub async fn discard_session(&self, session: &str) -> Result<usize, OutboxError> {
        (self.0)
            .discard_streams(StreamKind::MediaKey, &format!("{session}/"), SESSION_ENDED)
            .await
    }
}

impl OutboxSlot {
    fn media_keys(&self) -> Option<MediaKeys> {
        self.current.read().clone().map(MediaKeys)
    }

    fn install_media_key_carrier(&self, carrier: Arc<dyn MediaKeyCarrier>) {
        (self.senders).install(StreamKind::MediaKey, Arc::new(MediaKeySender(carrier)));
    }
}

impl Host {
    /// The unlocked identity's `media_key` streams, for the GUI's voice
    /// signaling (until E4.1 ports it).
    #[must_use]
    pub fn media_keys(&self) -> Option<MediaKeys> {
        self.ctx.outbox.media_keys()
    }

    /// Install the session's voice side as the `media_key` sender; the
    /// teardown removes it.
    pub fn install_media_key_carrier(&self, carrier: Arc<dyn MediaKeyCarrier>) {
        self.ctx.outbox.install_media_key_carrier(carrier);
    }
}
