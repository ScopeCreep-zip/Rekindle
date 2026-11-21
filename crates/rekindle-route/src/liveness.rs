//! Route liveness (plan RH1r-c; live finding 75; production research
//! batch 12): the rules that keep a silently dead private route from
//! swallowing traffic, as pure state machines over a monotonic clock. The
//! transport drives them; every jitter is drawn by the caller from the
//! RNG port and handed in, so a seeded run fixes the schedule.
//!
//! Veilid tells us of a dead own route only through `RouteChange`, and it
//! drops its own loopback failures while its online detector says Offline
//! (`routing_table/tasks/ping_validator.rs:666-690` at the pinned fork), so
//! a published route can sit dead for good. An `app_message` is a
//! statement: the sender sees only a local send error
//! (`veilid_api/routing_context.rs:318-341`). Hence two sides:
//! - [`InboundLiveness`], the owner: a round-trip self-test of each
//!   published inbound class (O1), a silence trigger (O2), heal pacing
//!   (O3) and the network-change trigger (O5);
//! - [`PeerLiveness`], a sender: suspect a peer's route from missing acks
//!   (S1, S2), re-fetch it at a bounded rate (S3), escalate (S4), reset on
//!   any authenticated frame (S5).

use std::time::Duration;

use tokio::time::Instant;

/// The self-test period, before its jitter (I2P `TestJob` `TEST_DELAY`,
/// `TestJob.java#L47`, `#L239`; batch 12 O1).
pub const SELF_TEST_PERIOD: Duration = Duration::from_secs(40);
/// The self-test jitter's bound: the period is 40 s + U(0, 13.3 s)
/// (`TestJob.java#L239`).
pub const SELF_TEST_JITTER_MS: u64 = 13_300;
/// No self-test while inbound arrived over the route this recently (I2P
/// `TestJob.java#L84-92`; Cwtch's idle gate, `networkCheck.go` L58-115).
pub const IDLE_GATE: Duration = Duration::from_secs(40);
/// A self-test probe's deadline (I2P `getTestPeriod`, 15 s by default,
/// `TestJob.java#L241-276`).
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
/// The retest after a failed self-test: 10 s + U(0, 10 s)
/// (`TestJob.java#L241-276`).
pub const RETEST_MIN: Duration = Duration::from_secs(10);
/// The retest jitter's bound, ms.
pub const RETEST_JITTER_MS: u64 = 10_000;
/// The least gap between two self-tests run on a peer's report that our
/// route failed it (plan LF6, lead decision 39): the report is evidence,
/// not a verdict, so it only brings the test forward, at most once per
/// retest interval, whoever sends it.
pub const REPORT_TEST_GAP: Duration = RETEST_MIN;
/// Consecutive failed self-tests that declare a route dead (I2P
/// `MAX_CONSECUTIVE_TEST_FAILURES`, `TunnelCreatorConfig.java#L62`).
pub const DEAD_AFTER_FAILURES: u32 = 3;
/// Inbound silence, while peers should be sending, that runs a self-test
/// at once: 2 × I2P's `REPLY_REQUEST_INTERVAL` (batch 12 O2, a pairing).
pub const SILENCE_TRIGGER: Duration = Duration::from_secs(120);
/// The debounce before a republish (I2P `PUBLISH_DELAY`,
/// `KademliaNetworkDatabaseFacade.java#L868-912`).
pub const PUBLISH_DELAY: Duration = Duration::from_secs(3);
/// The least gap between two republishes (I2P
/// `REPUBLISH_LEASESET_TIMEOUT`; Arti `UPLOAD_RATE_LIM_THRESHOLD`,
/// `publish/reactor.rs#L18-31`).
pub const PUBLISH_GAP: Duration = Duration::from_secs(60);
/// A failed republish retries after 20 s + U(0, 20 s) (I2P `RETRY_DELAY`,
/// `RepublishLeaseSetJob.java#L23-111`).
pub const PUBLISH_RETRY_MIN: Duration = Duration::from_secs(20);
/// The publish retry jitter's bound, ms.
pub const PUBLISH_RETRY_JITTER_MS: u64 = 20_000;

/// The ack deadline on a route that has answered before (I2P
/// `RATCHET_REPLY_TIMEOUT_MS_MIN`, `OutboundClientMessageOneShotJob.java`
/// `#L1199-1201`; batch 12 S1).
pub const ACK_TIMEOUT_PROVEN: Duration = Duration::from_secs(30);
/// The ack deadline on a route not yet proven (I2P `REPLY_TIMEOUT_MS_MIN`).
pub const ACK_TIMEOUT_NEW: Duration = Duration::from_secs(55);
/// Unacked sends that make a route suspect (I2P streaming
/// `DEFAULT_MAX_SENDS`, `ConnectionOptions.java#L88-134`; batch 12 S2).
pub const SUSPECT_AFTER_SENDS: u32 = 8;
/// How long a suspected route stays in the failure cache (Tor
/// `HS_CACHE_CLIENT_INTRO_STATE_MAX_AGE`, `hs_cache.h#L22-24`).
pub const FAILURE_CACHE: Duration = Duration::from_secs(120);
/// At most one re-fetch of a peer's route per this (batch 12 S3, matched
/// to the owner's [`PUBLISH_GAP`]).
pub const REFETCH_GAP: Duration = Duration::from_secs(60);
/// Suspect rounds after which the peer shows as unreachable (Arti
/// `default_hs_max_attempts`, `tor-circmgr/src/config.rs#L220-241`).
pub const UNREACHABLE_AFTER_ROUNDS: u32 = 6;
/// The fast-path backoff while unreachable: 1 min growing ×1.2 to 10 min
/// (Briar `TorPluginFactory.java` L45-47, `BackoffImpl.java` L26-31).
pub const UNREACHABLE_BACKOFF_MIN: Duration = Duration::from_secs(60);
/// The fast-path backoff's cap.
pub const UNREACHABLE_BACKOFF_MAX: Duration = Duration::from_secs(600);

/// A self-test's outcome, for the owner to act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfTest {
    /// The route answered.
    Healthy,
    /// It failed; it is retested soon.
    Retry,
    /// It failed [`DEAD_AFTER_FAILURES`] times running (or once right after
    /// a network change, O5): replace it.
    Dead,
}

/// The owner's view of one published inbound route class.
#[derive(Debug, Clone)]
pub struct InboundLiveness {
    /// When inbound last arrived over the route (or the route came up).
    last_inbound: Instant,
    /// When the last silence-triggered test ran.
    last_silence_test: Option<Instant>,
    next_test: Instant,
    failures: u32,
    /// The next test follows a network change: one failure is death (O5).
    after_change: bool,
    last_publish: Option<Instant>,
    /// Whether inbound over the route skips its scheduled test (the idle
    /// gate). Media keeps it; General and Sending do not (plan LF6).
    idle_gated: bool,
    /// When the last test brought forward by a peer's report fell due.
    last_report: Option<Instant>,
}

impl InboundLiveness {
    /// A route that came up at `now`; its first test is a period plus
    /// `jitter` away. Inbound over it skips a scheduled test (the idle
    /// gate, I2P `TestJob.java#L84-92`).
    #[must_use]
    pub fn new(now: Instant, jitter: Duration) -> Self {
        Self {
            last_inbound: now,
            last_silence_test: None,
            next_test: now + SELF_TEST_PERIOD + jitter,
            failures: 0,
            after_change: false,
            last_publish: None,
            idle_gated: true,
            last_report: None,
        }
    }

    /// As [`Self::new`], but tested every period whatever arrives over it
    /// (plan LF6, lead decision 39): inbound does not prove a route. On
    /// Linux round 20 the General route kept passing some `app_message`s
    /// while every peer's `app_call` to it timed out for ~3.5 min, and the
    /// idle gate skipped each test. Veilid's own gate has the same blind
    /// spot: any question or statement received marks the route valid
    /// (0.5.8 `route_stats.rs:475-485`) and a valid route is not
    /// loopback-tested for 30 s (`route_stats.rs:144-157`).
    #[must_use]
    pub fn ungated(now: Instant, jitter: Duration) -> Self {
        Self {
            idle_gated: false,
            ..Self::new(now, jitter)
        }
    }

    /// A new route replaced the old one at `now`: its tests start over;
    /// the publish pacing and the gating carry on.
    pub fn route_replaced(&mut self, now: Instant, jitter: Duration) {
        let last_publish = self.last_publish;
        let idle_gated = self.idle_gated;
        *self = Self::new(now, jitter);
        self.last_publish = last_publish;
        self.idle_gated = idle_gated;
    }

    /// A peer reported at `now` that our route failed it (its suspect
    /// round's probe reached us, plan RH1r-c S2): the next test falls due
    /// at once, unless a retest is already scheduled or a report brought
    /// one forward less than [`REPORT_TEST_GAP`] ago. Returns whether it
    /// did.
    pub fn reported(&mut self, now: Instant) -> bool {
        if self.failures > 0
            || self
                .last_report
                .is_some_and(|at| now.duration_since(at) < REPORT_TEST_GAP)
        {
            return false;
        }
        self.last_report = Some(now);
        self.next_test = self.next_test.min(now);
        true
    }

    /// Inbound arrived over the route.
    pub fn heard(&mut self, now: Instant) {
        self.last_inbound = now;
    }

    /// The network reconnected or changed class (O5): test at once, and a
    /// failure of that test is death.
    pub fn network_changed(&mut self, now: Instant) {
        self.next_test = now;
        self.after_change = true;
    }

    /// Whether to self-test now. `expecting` is whether peers should be
    /// sending to us (O2). A scheduled test of a busy route is skipped and
    /// rescheduled `jitter()` past the period (the idle gate); the jitter is
    /// drawn only then.
    pub fn should_test(
        &mut self,
        now: Instant,
        expecting: bool,
        jitter: impl FnOnce() -> Duration,
    ) -> bool {
        if expecting
            && self.failures == 0
            && now.duration_since(self.last_inbound) >= SILENCE_TRIGGER
            && self
                .last_silence_test
                .is_none_or(|at| now.duration_since(at) >= SILENCE_TRIGGER)
        {
            self.last_silence_test = Some(now);
            return true;
        }
        if now < self.next_test {
            return false;
        }
        if self.idle_gated
            && self.failures == 0
            && !self.after_change
            && now.duration_since(self.last_inbound) < IDLE_GATE
        {
            self.next_test = now + SELF_TEST_PERIOD + jitter();
            return false;
        }
        true
    }

    /// The test that ran at `now` came back `ok`. `period_jitter` and
    /// `retest_jitter` are drawn below [`SELF_TEST_JITTER_MS`] and
    /// [`RETEST_JITTER_MS`].
    pub fn tested(
        &mut self,
        now: Instant,
        ok: bool,
        period_jitter: Duration,
        retest_jitter: Duration,
    ) -> SelfTest {
        let after_change = std::mem::take(&mut self.after_change);
        if ok {
            self.failures = 0;
            self.next_test = now + SELF_TEST_PERIOD + period_jitter;
            return SelfTest::Healthy;
        }
        self.failures += 1;
        if after_change || self.failures >= DEAD_AFTER_FAILURES {
            self.failures = 0;
            self.next_test = now + SELF_TEST_PERIOD + period_jitter;
            return SelfTest::Dead;
        }
        self.next_test = now + RETEST_MIN + retest_jitter;
        SelfTest::Retry
    }

    /// When a publish asked for at `now` may go: the first at once; a
    /// republish [`PUBLISH_DELAY`] away and [`PUBLISH_GAP`] after the last
    /// one (O3).
    #[must_use]
    pub fn publish_at(&self, now: Instant) -> Instant {
        self.last_publish
            .map_or(now, |last| (now + PUBLISH_DELAY).max(last + PUBLISH_GAP))
    }

    /// A republish went out at `now` (landed or not, it counts against
    /// the gap).
    pub fn published(&mut self, now: Instant) {
        self.last_publish = Some(now);
    }
}

/// A sender's view of one peer's route.
#[derive(Debug, Clone, Default)]
pub struct PeerLiveness {
    /// The first send since the peer was last heard.
    first_unacked: Option<Instant>,
    unacked: u32,
    /// The peer has been heard over this route before.
    proven: bool,
    rounds: u32,
    last_refetch: Option<Instant>,
    /// The highest route record seq read for the peer.
    seen_seq: Option<u32>,
    /// The route blob we suspected, and when (the failure cache).
    failed: Option<(Vec<u8>, Instant)>,
    unreachable: bool,
    /// While unreachable, the next fast-path attempt and the backoff.
    next_attempt: Option<Instant>,
    backoff: Duration,
}

/// A re-read route record, against the seqs seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetched {
    /// A strictly higher seq: the owner republished.
    Newer,
    /// The record we hold.
    Same,
    /// Older than one seen: a lagging replica's.
    Stale,
}

/// What a send to a suspect route should do first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suspicion {
    /// Nothing: the route is not suspect, or a re-fetch ran too recently.
    None,
    /// Drop the cached route and re-read the peer's route record (S2).
    Refetch,
}

impl PeerLiveness {
    /// A send that expects an answer went out at `now`.
    pub fn sent(&mut self, now: Instant) {
        self.unacked = self.unacked.saturating_add(1);
        self.first_unacked.get_or_insert(now);
    }

    /// An authenticated frame or ack from the peer arrived (S5). Returns
    /// whether the peer was shown unreachable until now.
    pub fn heard(&mut self) -> bool {
        let was = self.unreachable;
        *self = Self {
            proven: true,
            last_refetch: self.last_refetch,
            seen_seq: self.seen_seq,
            ..Self::default()
        };
        was
    }

    /// Whether the route is suspect at `now` (S2): one ack deadline passed
    /// since the first unacked send, or [`SUSPECT_AFTER_SENDS`] went
    /// unacked.
    #[must_use]
    pub fn suspect(&self, now: Instant) -> bool {
        let deadline = if self.proven {
            ACK_TIMEOUT_PROVEN
        } else {
            ACK_TIMEOUT_NEW
        };
        self.unacked >= SUSPECT_AFTER_SENDS
            || self
                .first_unacked
                .is_some_and(|first| now.duration_since(first) >= deadline)
    }

    /// What to do before a send at `now`: a re-fetch at most once per
    /// [`REFETCH_GAP`] (S3).
    #[must_use]
    pub fn before_send(&self, now: Instant) -> Suspicion {
        if !self.suspect(now) {
            return Suspicion::None;
        }
        if self
            .last_refetch
            .is_some_and(|at| now.duration_since(at) < REFETCH_GAP)
        {
            return Suspicion::None;
        }
        Suspicion::Refetch
    }

    /// A suspect round began at `now` against the route `blob` we held:
    /// it is failure-cached for [`FAILURE_CACHE`] and the counters start
    /// over. Returns `true` when this round makes the peer unreachable (S4).
    pub fn begin_round(&mut self, now: Instant, blob: Option<Vec<u8>>) -> bool {
        self.last_refetch = Some(now);
        self.rounds += 1;
        self.unacked = 0;
        self.first_unacked = None;
        self.failed = blob.map(|blob| (blob, now));
        if self.rounds >= UNREACHABLE_AFTER_ROUNDS && !self.unreachable {
            self.unreachable = true;
            self.backoff = UNREACHABLE_BACKOFF_MIN;
            self.next_attempt = Some(now + self.backoff);
            return true;
        }
        false
    }

    /// A re-read of the peer's route record found `seq` (S2): only a
    /// strictly higher seq than any seen is a new record; the same seq is
    /// the record we hold (probe it once); a lower one is a lagging
    /// replica's, ignored.
    pub fn observe(&mut self, seq: u32) -> Fetched {
        match self.seen_seq {
            Some(seen) if seq < seen => Fetched::Stale,
            Some(seen) if seq == seen => Fetched::Same,
            _ => {
                self.seen_seq = Some(seq);
                Fetched::Newer
            }
        }
    }

    /// Whether `blob` is the route we suspected within [`FAILURE_CACHE`]:
    /// not to be sent to again until a probe of it answers.
    #[must_use]
    pub fn failure_cached(&self, now: Instant, blob: &[u8]) -> bool {
        self.failed
            .as_ref()
            .is_some_and(|(failed, at)| failed == blob && now.duration_since(*at) < FAILURE_CACHE)
    }

    /// A probe of the suspected route answered: it is usable again.
    pub fn clear_failure(&mut self) {
        self.failed = None;
    }

    /// Whether the fast path may be tried at `now`: always while the peer
    /// is reachable; once per backoff step while it is not (S4), the
    /// durable chain carrying the rest.
    pub fn fast_path_allowed(&mut self, now: Instant) -> bool {
        if !self.unreachable {
            return true;
        }
        if self.next_attempt.is_some_and(|at| now < at) {
            return false;
        }
        let grown = self.backoff.mul_f64(1.2);
        self.backoff = grown.clamp(UNREACHABLE_BACKOFF_MIN, UNREACHABLE_BACKOFF_MAX);
        self.next_attempt = Some(now + self.backoff);
        true
    }

    /// Whether a send to the peer awaits its answer.
    #[must_use]
    pub fn awaiting(&self) -> bool {
        self.unacked > 0
    }

    /// Whether the peer shows as unreachable.
    #[must_use]
    pub fn unreachable(&self) -> bool {
        self.unreachable
    }
}

#[cfg(test)]
#[path = "liveness_tests.rs"]
mod tests;
