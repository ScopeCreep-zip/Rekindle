//! Stream kinds (plan E2.5): each send belongs to one ordered stream
//! `<kind>:<scope>`, and each kind has its retry policy. `signaling` comes
//! at E4.1.

use std::time::Duration;

use rekindle_types::message::{CALL_TIMEOUT_MS, DM_ACK_DELAY_MAX_MS};
use rekindle_utils::random::RandomSource;

/// The shortest wait before a `dm` row is sent again: the longest a
/// peer's `DmAck` may take to come back (live A/B finding 49).
pub const DM_RETRY_FLOOR: Duration = Duration::from_millis(DM_ACK_DELAY_MAX_MS + CALL_TIMEOUT_MS);

/// What a stream carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamKind {
    /// 1:1 messages to one peer.
    Dm,
    /// A friend request, accept or removal to one peer.
    FriendHandshake,
    /// An own-record DHT write (the C7.6h `dht_outbox`).
    DhtWrite,
    /// A community channel post (C7.14).
    ChannelPost,
    /// A call media key, sealed, to one recipient of one channel session
    /// (C7.22): the session's end discards the session's rows, so the
    /// horizon is only an upper bound.
    MediaKey,
    /// Our group-DM content to one group (plan E2.6): a row settles `sent`
    /// when its entry lands on our group chain; no member acknowledges it.
    GroupDm,
    /// Our signed governance ops for one community (plan E3.2, D22): a row
    /// is one encoded op, appended to our member record's op log (`M`/2,
    /// through the chain primitive); it settles `sent` when the log's head
    /// names it. No member acknowledges an op.
    GovernanceOp,
}

impl StreamKind {
    const ALL: [Self; 7] = [
        Self::Dm,
        Self::GroupDm,
        Self::GovernanceOp,
        Self::FriendHandshake,
        Self::DhtWrite,
        Self::ChannelPost,
        Self::MediaKey,
    ];

    /// The stored form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dm => "dm",
            Self::FriendHandshake => "friend_handshake",
            Self::DhtWrite => "dht_write",
            Self::ChannelPost => "channel_post",
            Self::MediaKey => "media_key",
            Self::GroupDm => "group_dm",
            Self::GovernanceOp => "governance_op",
        }
    }

    /// The kind a stored form names.
    #[must_use]
    pub fn parse(stored: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == stored)
    }

    /// The stream of this kind for `scope` (a peer, a record, a channel).
    #[must_use]
    pub fn stream(self, scope: &str) -> String {
        format!("{}:{scope}", self.as_str())
    }

    /// Whether a row stays unsettled after it is sent, until the peer
    /// acknowledges it (and is re-sent meanwhile). A DHT write and a
    /// channel post settle when the write reaches consensus; a friend
    /// handshake frame is a control frame no `DmAck` answers, so it settles
    /// `sent` once it leaves (or `delivered` on an `app_call` receipt), and
    /// the one that waits says so per row ([`crate::AttemptOutcome::Awaiting`]).
    #[must_use]
    pub fn awaits_ack(self) -> bool {
        matches!(self, Self::Dm | Self::MediaKey)
    }

    /// Whether the user may Retry a failed row of this kind. A call media
    /// key may not: the session's end discards it, and a key is never
    /// given a fresh deadline past the call it keys.
    #[must_use]
    pub fn user_retryable(self) -> bool {
        !matches!(self, Self::MediaKey)
    }

    /// This kind's default retry policy.
    #[must_use]
    pub fn policy(self) -> OutboxRetry {
        let (base, cap) = match self {
            // The record pool's write of the slot is the send (plan E2.5).
            Self::ChannelPost => return OutboxRetry::SettledExternally,
            // A group send and an op append are chain writes, retried
            // like a DM.
            // A DM's retry never comes before its ack can (live A/B
            // finding 49): the peer's `DmAck` leaves up to
            // `DM_ACK_DELAY_MAX_MS` after the entry, and its send may take
            // one call timeout.
            Self::Dm => {
                return OutboxRetry::Backoff {
                    base: Duration::from_secs(1),
                    cap: Duration::from_secs(300),
                    floor: DM_RETRY_FLOOR,
                }
            }
            Self::GroupDm | Self::GovernanceOp => (1, 300),
            Self::FriendHandshake => (2, 900),
            Self::DhtWrite => (1, 60),
            // C7.22's 1/2/4/8/15 s: a call key is useless late.
            Self::MediaKey => (1, 15),
        };
        OutboxRetry::Backoff {
            base: Duration::from_secs(base),
            cap: Duration::from_secs(cap),
            floor: Duration::ZERO,
        }
    }
}

/// When a row is attempted again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxRetry {
    /// Exponential backoff from `base`, capped at `cap`, with equal jitter
    /// (half the step fixed, half uniform), so peers that failed together
    /// do not retry together; each wait is that step after `floor`.
    Backoff {
        base: Duration,
        cap: Duration,
        floor: Duration,
    },
    /// Never attempted by the engine: no sender, no attempt, no backoff
    /// pass. Only the host's settle of the stream
    /// ([`crate::Outbox::settle_stream`]) or the row's deadline settles it
    /// (a `channel_post`, whose send is the record pool's write).
    SettledExternally,
}

impl OutboxRetry {
    /// The wait after the `attempts`-th attempt (1 for the first); `None`
    /// for a policy the engine never attempts.
    #[must_use]
    pub fn delay(self, attempts: u32, rng: &dyn RandomSource) -> Option<Duration> {
        match self {
            Self::SettledExternally => None,
            Self::Backoff { base, cap, floor } => {
                let step = base
                    .saturating_mul(1u32 << attempts.saturating_sub(1).min(20))
                    .min(cap);
                let step_ms = u64::try_from(step.as_millis()).unwrap_or(u64::MAX);
                let half = step_ms / 2;
                Some(floor + Duration::from_millis(half + rng.below(step_ms - half + 1)))
            }
        }
    }
}

#[cfg(test)]
mod group_dm_tests {
    use super::{OutboxRetry, StreamKind};

    /// A group send settles on its chain write: no ack, retryable, backed off.
    #[test]
    fn a_group_dm_row_awaits_no_ack_and_may_be_retried() {
        let kind = StreamKind::GroupDm;
        assert_eq!(StreamKind::parse(kind.as_str()), Some(kind));
        assert_eq!(kind.stream("ab"), "group_dm:ab");
        assert!(!kind.awaits_ack());
        assert!(kind.user_retryable());
        assert!(matches!(kind.policy(), OutboxRetry::Backoff { .. }));
    }

    /// An op append settles on its log write, like a group send.
    #[test]
    fn a_governance_op_row_awaits_no_ack_and_may_be_retried() {
        let kind = StreamKind::GovernanceOp;
        assert_eq!(StreamKind::parse(kind.as_str()), Some(kind));
        assert_eq!(kind.stream("c"), "governance_op:c");
        assert!(!kind.awaits_ack());
        assert!(kind.user_retryable());
        assert!(matches!(kind.policy(), OutboxRetry::Backoff { .. }));
    }
}
