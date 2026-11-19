//! Stream kinds (plan E2.5): each send belongs to one ordered stream
//! `<kind>:<scope>`, and each kind has its retry policy. `signaling` comes
//! at E4.1.

use std::time::Duration;

use rekindle_types::message::{CALL_TIMEOUT_MS, DM_ACK_DELAY_MAX_MS};
use rekindle_utils::random::RandomSource;

/// The shortest wait before a `dm` row is sent again: the longest a
/// peer's `DmAck` may take to come back (live A/B finding 49).
pub const DM_RETRY_FLOOR: Duration = Duration::from_millis(DM_ACK_DELAY_MAX_MS + CALL_TIMEOUT_MS);

/// The shortest wait before a `friend_handshake` row is sent again: the
/// longest its answer may take to come back. The peer answers at once (an
/// `UnfriendedAck`, an accept) from its own outbox, so the round trip is
/// two app messages, bounded by the call timeout `CALL_TIMEOUT_MS`
/// (Veilid's `rpc.timeout_ms`). A shorter retry re-sends an `Unfriended`
/// whose ack is still on its way into the session the peer just deleted
/// (netsim `removal_ack`, RV5: the 1-2 s first step came before a 1.4 s
/// round trip).
pub const HANDSHAKE_RETRY_FLOOR: Duration = Duration::from_millis(CALL_TIMEOUT_MS);

/// When a `dm` row that reached `sent` (its entry landed, or its
/// `StreamRecord` left) is first asked for again if no ack covers it
/// (register 2026-10-24): two of the longest an ack may take, its 2–10 s
/// coalescing delay plus the call round trip, whose p99 is bounded by the
/// call timeout `CALL_TIMEOUT_MS`.
pub const DM_FIRST_REASK: Duration =
    Duration::from_millis(2 * (DM_ACK_DELAY_MAX_MS + CALL_TIMEOUT_MS));

/// How long a handed-off chain append is waited for before its outcome is
/// unknown (live A/B finding 73; research `prod_research_batch9.md` Q1:
/// Signal bounds an attempt at 10 s, SimpleX at 30 s, and neither reads a
/// timeout as "not stored"). It is [`DM_RETRY_FLOOR`], the longest an ack
/// may take, half of [`DM_FIRST_REASK`]: so the first re-ask always finds
/// the append past it, and a row whose append is still running is
/// re-hinted (the hint only) instead of waiting on the DHT. The append is
/// never cancelled: its stream stays busy until it ends (one append in
/// flight per stream), and its late outcome is recorded as any landing is
/// (`OutboxDeps::land`, a no-op on a row already landed).
pub const DM_APPEND_DEADLINE: Duration = DM_RETRY_FLOOR;

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
    /// A community key sealed to one member (`MekTransfer`, plan E3.2
    /// D16: the inviter's push to a joiner), on the stream of
    /// `<community>:<peer>`: an `app_call` whose `MekTransferAck` reply
    /// settles it `delivered`, retried until its deadline; the peer's
    /// departure, or ours, discards the stream's rows.
    MekTransfer,
    /// Our `MemberJoined` handed to a member (`BootstrapRequest @42`,
    /// plan E3.2 join step 5), on the stream of `<community>`: a joiner
    /// whose members were all away when it joined is known to nobody but
    /// itself, since every member's sweep and ring read only records the
    /// member already knows (ADR 0011 "joining works with nobody online").
    /// An `app_call` to the inviter's current route whose
    /// `bootstrapResponseV2` (the inviter merged the op) settles it
    /// `delivered`; once one member merged it, its frontier cites the op
    /// and every other member's sweep finds it. Our leave discards it.
    Bootstrap,
}

impl StreamKind {
    const ALL: [Self; 9] = [
        Self::Dm,
        Self::GroupDm,
        Self::GovernanceOp,
        Self::FriendHandshake,
        Self::DhtWrite,
        Self::ChannelPost,
        Self::MediaKey,
        Self::MekTransfer,
        Self::Bootstrap,
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
            Self::MekTransfer => "mek_transfer",
            Self::Bootstrap => "bootstrap",
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

    /// How long a row of this kind that just reached `sent` waits for the
    /// peer's ack before it is first asked for again; `None`: its backoff.
    /// Drawn uniformly from `[DM_FIRST_REASK, 1.5 × DM_FIRST_REASK]` by
    /// `rng` (threat model: no deterministic timing; the same equal-jitter
    /// half step as [`OutboxRetry::delay`]), never below the floor an ack
    /// needs.
    #[must_use]
    pub fn first_reask(self, rng: &dyn RandomSource) -> Option<Duration> {
        let floor = u64::try_from(DM_FIRST_REASK.as_millis()).unwrap_or(u64::MAX);
        matches!(self, Self::Dm).then(|| Duration::from_millis(floor + rng.below(floor / 2 + 1)))
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
            // A group send, an op append and a channel post (plan E3.3,
            // our `M`/4 outbox) are chain writes, retried like a DM.
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
            Self::GroupDm | Self::GovernanceOp | Self::ChannelPost => (1, 300),
            // A key transfer is a receipted control call, like a
            // handshake frame: its peer may be offline for a while.
            Self::FriendHandshake | Self::MekTransfer => {
                return OutboxRetry::Backoff {
                    base: Duration::from_secs(2),
                    cap: Duration::from_secs(900),
                    floor: HANDSHAKE_RETRY_FLOOR,
                }
            }
            Self::DhtWrite => (1, 60),
            // A receipted control call whose peer may be away for a day;
            // capped at the anti-entropy sweep's 60 s period
            // (`rekindle_governance_runtime::anti_entropy::SWEEP_PERIOD`,
            // the bound on how late a member learns any op), so a member
            // back from a long absence learns the join within a sweep.
            Self::Bootstrap => {
                return OutboxRetry::Backoff {
                    base: Duration::from_secs(2),
                    cap: Duration::from_secs(60),
                    floor: HANDSHAKE_RETRY_FLOOR,
                }
            }
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
}

impl OutboxRetry {
    /// The wait after the `attempts`-th attempt (1 for the first).
    #[must_use]
    pub fn delay(self, attempts: u32, rng: &dyn RandomSource) -> Duration {
        let Self::Backoff { base, cap, floor } = self;
        let step = base
            .saturating_mul(1u32 << attempts.saturating_sub(1).min(20))
            .min(cap);
        let step_ms = u64::try_from(step.as_millis()).unwrap_or(u64::MAX);
        let half = step_ms / 2;
        floor + Duration::from_millis(half + rng.below(step_ms - half + 1))
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

    /// A key transfer settles on its receipt: no ack row, retryable,
    /// backed off like a handshake frame.
    #[test]
    fn a_mek_transfer_row_settles_on_its_receipt_and_backs_off_like_a_handshake() {
        let kind = StreamKind::MekTransfer;
        assert_eq!(StreamKind::parse("mek_transfer"), Some(kind));
        assert_eq!(kind.stream("c:p"), "mek_transfer:c:p");
        assert!(!kind.awaits_ack());
        assert_eq!(kind.first_reask(&rekindle_utils::OsRandom), None);
        assert!(kind.user_retryable());
        assert_eq!(kind.policy(), StreamKind::FriendHandshake.policy());
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
