//! Route liveness (plan RH1r-c; live finding 75; production research
//! batch 12): the rules that keep a silently dead private route from
//! swallowing traffic, as pure state machines over a monotonic clock. The
//! transport drives them; every jitter is drawn by the caller from the
//! RNG port and handed in, so a seeded run fixes the schedule.
//!
//! Veilid tests our allocated routes itself, a loopback keepalive every
//! 10 s (veilid-core 0.5.8 `private_route_management.rs:7`), and reports a
//! death through `RouteChange.dead_routes`, which `OwnRoutes::on_dead`
//! already handles (plan RV1). What a loopback cannot see is a route dead
//! only from outside, so the owner acts on evidence instead of testing:
//! - [`InboundLiveness`], the owner: rotate the General route on inbound
//!   silence while a peer should be answering, on a peer's `'L'` report,
//!   or when a friend's DM chain runs ahead of the hints that should have
//!   reached us over the route; pace the republish (O3). Each rotation
//!   draws a new route and so spends from the new-route budget
//!   ([`crate::budget`]): a forged-signal storm stops at its cap;
//! - [`PeerLiveness`], a sender: suspect a peer's route from missing acks
//!   (S1, S2), re-fetch it at a bounded rate (S3), escalate (S4), reset on
//!   any authenticated frame (S5).

use std::time::Duration;

use tokio::time::Instant;

/// Inbound silence on our General route, while a peer should be
/// answering us, that rotates it: 2 × I2P's `REPLY_REQUEST_INTERVAL`
/// (batch 12 O2, a pairing). A route younger than this is never rotated for
/// silence: peers may still hold the blob it replaced.
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

/// The least age of our General route before a peer's `'L'` report
/// rotates it (plan RV1): a report is evidence, but peers that suspected
/// the old route keep probing until their re-fetch (at most one per
/// [`REFETCH_GAP`]) finds the new blob, so a report on a route this young
/// is about the one it replaced.
pub const REPORT_ROTATE_AGE: Duration = REFETCH_GAP;

/// How long after a friend's DM chain shows a new entry its hint may still
/// arrive over our General route (plan RV1): the sender appends before the
/// hint leaves (`rekindle-transport` `dm::stream`), so a chain entry whose
/// hint is still missing one proven ack deadline later is evidence the
/// hint was lost on our route.
pub const HINT_WINDOW: Duration = ACK_TIMEOUT_PROVEN;

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

/// The owner's view of our General route (plan RV1): when to rotate it on
/// evidence that it is dead from outside, and when to republish.
#[derive(Debug, Clone)]
pub struct InboundLiveness {
    /// When inbound last arrived over the route (or the route came up).
    last_inbound: Instant,
    /// When the live route came up.
    came_up: Instant,
    last_publish: Option<Instant>,
    /// Per friend, when their last hint arrived over the route.
    hints: std::collections::HashMap<String, Instant>,
    /// Per friend, when their chain first showed an entry no hint carried.
    ahead: std::collections::HashMap<String, Instant>,
}

impl InboundLiveness {
    /// A route that came up at `now`.
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            last_inbound: now,
            came_up: now,
            last_publish: None,
            hints: std::collections::HashMap::new(),
            ahead: std::collections::HashMap::new(),
        }
    }

    /// A new route replaced the old one at `now`: its evidence starts over;
    /// the publish pacing carries on.
    pub fn route_replaced(&mut self, now: Instant) {
        let last_publish = self.last_publish;
        *self = Self::new(now);
        self.last_publish = last_publish;
    }

    /// A peer reported at `now` that our route failed it (its suspect
    /// round's `'L'` probe reached us, plan RH1r-c S2): whether to rotate
    /// the route. The reporter is a peer online now; a report on a route
    /// younger than [`REPORT_ROTATE_AGE`] is about the route it replaced.
    #[must_use]
    pub fn reported(&self, now: Instant) -> bool {
        now.duration_since(self.came_up) >= REPORT_ROTATE_AGE
    }

    /// A hint from friend `peer` arrived over the route at `now`: their
    /// chain is not ahead of it.
    pub fn hinted(&mut self, peer: &str, now: Instant) {
        self.hints.insert(peer.to_owned(), now);
        self.ahead.remove(peer);
    }

    /// Friend `peer`'s DM chain showed a new entry at `now` (its watch
    /// fired): ahead of their hints unless one arrived within
    /// [`HINT_WINDOW`].
    pub fn chain_changed(&mut self, peer: &str, now: Instant) {
        let hinted = self
            .hints
            .get(peer)
            .is_some_and(|at| now.duration_since(*at) < HINT_WINDOW);
        if hinted {
            self.ahead.entry(peer.to_owned()).or_insert(now);
        }
    }

    /// Whether a friend's DM chain has run ahead of its hints at `now`: an
    /// entry whose hint has not arrived within [`HINT_WINDOW`], on a route
    /// old enough that the friend holds its blob ([`REPORT_ROTATE_AGE`]).
    #[must_use]
    pub fn chain_ahead(&self, now: Instant) -> bool {
        now.duration_since(self.came_up) >= REPORT_ROTATE_AGE
            && self
                .ahead
                .values()
                .any(|at| now.duration_since(*at) >= HINT_WINDOW)
    }

    /// Inbound arrived over the route.
    pub fn heard(&mut self, now: Instant) {
        self.last_inbound = now;
    }

    /// Whether inbound silence rotates the route at `now`: `expecting` is
    /// whether a peer should be answering us (a send to an online peer
    /// awaits its answer, O2), and nothing has arrived over the route, nor
    /// has it been up, for [`SILENCE_TRIGGER`].
    #[must_use]
    pub fn silent(&self, now: Instant, expecting: bool) -> bool {
        expecting && now.duration_since(self.last_inbound) >= SILENCE_TRIGGER
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
