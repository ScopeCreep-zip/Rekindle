//! The beat engine (plan E3.4 S2, "Cadence" and "Constant rate"): our beat
//! (`M`/1) in every community, Veilid-free. The Host's presence adapter
//! (`rekindle-node` `host/adapters/presence/`) gives it a [`BeatSink`] over
//! its record port, on both modes.
//!
//! - **Schedule.** Each community's next beat is drawn fresh from
//!   U([`BEAT_DELAY_MIN`], [`BEAT_DELAY_MAX`]), 2.5 to 7.5 minutes (mean
//!   5), independently per community, so one person's beats in two
//!   communities cannot be lined up by their timing. A beat says the status
//!   we hold when it falls due: auto-away waits for it.
//! - **Explicit actions.** A status the user picks ([`BeatEngine::status_changed`])
//!   is written at once to every community; leaving one writes its
//!   `Departed` beat ([`BeatEngine::depart`]) and ends its schedule. An
//!   explicit write does not move the schedule: beats go out at the same
//!   rate whatever the user does.
//! - **Voice** (plan E3.4 S8-pre): a voice join or leave
//!   ([`BeatEngine::refresh_community`]) writes that community's beat at once;
//!   the sink fills its `voiceChannel`.
//! - **Invisible.** A status that hides presence
//!   ([`crate::hides_presence`]) writes one `Offline` beat when picked (the
//!   same beat a quit writes, so Invisible reads as offline) and nothing
//!   after it, scheduled or not, until a visible status is picked.
//! - **Stop.** [`BeatEngine::run`] returns when its token is cancelled (the
//!   unlock's scope at lock); no scheduled write starts after that. The
//!   teardown then writes the quit's `Offline` beat ([`BeatEngine::quit`]).
//!
//! The ticks are tokio time, which the netsim pauses.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rekindle_codec::member_record::BeatState;
use rekindle_types::presence::SessionStatus;
use rekindle_utils::random::RandomSource;
use tokio::sync::{Mutex, Notify};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// The shortest delay to a community's next beat (2.5 min).
pub const BEAT_DELAY_MIN: Duration = Duration::from_millis(150_000);
/// The longest delay to a community's next beat (7.5 min).
pub const BEAT_DELAY_MAX: Duration = Duration::from_millis(450_000);
/// How often the engine looks for a community joined or left between beats:
/// a joined community's first scheduled beat is drawn within this of the
/// join's own beat, so it falls well inside the 15-minute liveness.
pub const MEMBERSHIP_CHECK: Duration = BEAT_DELAY_MIN;

/// A delay drawn uniformly, to the millisecond, from
/// [`BEAT_DELAY_MIN`]..=[`BEAT_DELAY_MAX`].
#[must_use]
pub fn beat_delay(rng: &dyn RandomSource) -> Duration {
    let min = millis(BEAT_DELAY_MIN);
    let span = millis(BEAT_DELAY_MAX) - min;
    Duration::from_millis(min + rng.below(span + 1))
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// What the engine writes through.
#[async_trait]
pub trait BeatSink: Send + Sync {
    /// The communities we beat in now: every membership with our member
    /// record.
    fn communities(&self) -> Vec<String>;
    /// The status we hold now.
    fn status(&self) -> SessionStatus;
    /// Write our beat saying `state` in `community`.
    ///
    /// # Errors
    /// The beat was not written.
    async fn write_beat(&self, community: &str, state: BeatState) -> Result<(), String>;
    /// Write an explicitly refreshed beat, then hint its new sequence.
    ///
    /// # Errors
    /// The beat was not written or its hint could not be encoded.
    async fn write_refreshed_beat(&self, community: &str, state: BeatState) -> Result<(), String>;
    /// Write our quit's `Offline` beat in `community` and tell its members
    /// at once (a `HeadAdvanced` hint, plan E3.4 "Constant rate").
    ///
    /// # Errors
    /// The beat was not written.
    async fn write_quit_beat(&self, community: &str) -> Result<(), String>;
}

#[derive(Default)]
struct Schedule {
    /// Each community's next scheduled beat.
    due: BTreeMap<String, Instant>,
    /// Communities whose `Departed` beat is written, until the membership
    /// is gone.
    departed: BTreeSet<String>,
    /// The `Offline` beat of a hiding status is written.
    hidden: bool,
}

/// The engine of one unlock.
pub struct BeatEngine {
    sink: Arc<dyn BeatSink>,
    rng: Arc<dyn RandomSource>,
    /// Held across every write, so a departure's beat is never followed by
    /// a scheduled one.
    schedule: Mutex<Schedule>,
    explicit: Notify,
}

impl BeatEngine {
    #[must_use]
    pub fn new(sink: Arc<dyn BeatSink>, rng: Arc<dyn RandomSource>) -> Self {
        Self {
            sink,
            rng,
            schedule: Mutex::new(Schedule::default()),
            explicit: Notify::new(),
        }
    }

    /// The user picked a status: the engine writes it at once.
    pub fn status_changed(&self) {
        self.explicit.notify_one();
    }

    /// Our session or consent changed in `community`: its beat is
    /// written at once, saying the status we hold (an explicit action, plan
    /// E3.4 "Constant rate"). Nothing is written while a hiding status
    /// holds, after our `Departed` beat, or in a community we hold no
    /// membership in.
    ///
    /// # Errors
    /// The beat was not written.
    pub async fn refresh_community(&self, community: &str) -> Result<(), String> {
        let mut schedule = self.schedule.lock().await;
        self.reconcile(&mut schedule);
        if schedule.hidden || !schedule.due.contains_key(community) {
            return Ok(());
        }
        match scheduled_state(self.sink.status()) {
            Some(state) => self.sink.write_refreshed_beat(community, state).await,
            None => Ok(()),
        }
    }

    /// Write our `Departed` beat in `community` (we are leaving it) and
    /// end its schedule.
    ///
    /// # Errors
    /// The beat was not written; the schedule is ended anyway.
    pub async fn depart(&self, community: &str) -> Result<(), String> {
        let mut schedule = self.schedule.lock().await;
        schedule.due.remove(community);
        schedule.departed.insert(community.to_owned());
        self.sink.write_beat(community, BeatState::Departed).await
    }

    /// Our quit, once [`Self::run`] returned: one `Offline` beat in every
    /// community we are a member of now, unless a hiding status already
    /// wrote it. The membership is read again here, not taken from the
    /// schedule alone: a community created or joined since the last
    /// membership check (up to [`MEMBERSHIP_CHECK`]) has no schedule yet,
    /// and its members would see the quit only at the beat's expiry.
    pub async fn quit(&self) {
        let mut schedule = self.schedule.lock().await;
        if std::mem::replace(&mut schedule.hidden, true) {
            tracing::info!("quit beats skipped: a hiding status already wrote Offline");
            return;
        }
        self.reconcile(&mut schedule);
        let communities: Vec<String> = schedule.due.keys().cloned().collect();
        tracing::info!(communities = communities.len(), "quit beats starting");
        let mut written = 0usize;
        for community in &communities {
            match self.sink.write_quit_beat(community).await {
                Ok(()) => written += 1,
                Err(error) => tracing::warn!(%error, "quit beat not written"),
            }
        }
        tracing::info!(
            written,
            failed = communities.len() - written,
            "quit beats finished"
        );
    }

    /// Run until `stop`.
    pub async fn run(&self, stop: CancellationToken) {
        loop {
            let wake_at = {
                let mut schedule = self.schedule.lock().await;
                self.reconcile(&mut schedule);
                let check = Instant::now() + MEMBERSHIP_CHECK;
                schedule
                    .due
                    .values()
                    .min()
                    .map_or(check, |due| check.min(*due))
            };
            tokio::select! {
                biased;
                () = stop.cancelled() => return,
                () = self.explicit.notified() => self.write_explicit(&stop).await,
                () = tokio::time::sleep_until(wake_at) => self.write_due(&stop).await,
            }
        }
    }

    /// Schedule the communities joined and drop the ones left.
    fn reconcile(&self, schedule: &mut Schedule) {
        let now: BTreeSet<String> = self.sink.communities().into_iter().collect();
        schedule.due.retain(|c, _| now.contains(c));
        schedule.departed.retain(|c| now.contains(c));
        for community in now {
            if !schedule.departed.contains(&community) && !schedule.due.contains_key(&community) {
                let due = Instant::now() + beat_delay(&*self.rng);
                schedule.due.insert(community, due);
            }
        }
    }

    /// Every beat due now, each rescheduled with a fresh draw.
    async fn write_due(&self, stop: &CancellationToken) {
        let mut schedule = self.schedule.lock().await;
        // A community left since the sleep began beats no more.
        self.reconcile(&mut schedule);
        let now = Instant::now();
        let due: Vec<String> = (schedule.due.iter())
            .filter(|(_, at)| **at <= now)
            .map(|(c, _)| c.clone())
            .collect();
        let state = scheduled_state(self.sink.status());
        for community in due {
            if stop.is_cancelled() {
                return;
            }
            if let Some(state) = state {
                self.write(&community, state).await;
            }
            let next = Instant::now() + beat_delay(&*self.rng);
            schedule.due.insert(community, next);
        }
    }

    /// The status the user picked, in every community.
    async fn write_explicit(&self, stop: &CancellationToken) {
        let mut schedule = self.schedule.lock().await;
        self.reconcile(&mut schedule);
        let status = self.sink.status();
        let state = if crate::hides_presence(status) {
            if schedule.hidden {
                return;
            }
            schedule.hidden = true;
            BeatState::Offline
        } else {
            schedule.hidden = false;
            status.into()
        };
        let communities: Vec<String> = schedule.due.keys().cloned().collect();
        for community in communities {
            if stop.is_cancelled() {
                return;
            }
            self.write(&community, state).await;
        }
    }

    async fn write(&self, community: &str, state: BeatState) {
        if let Err(error) = self.sink.write_beat(community, state).await {
            tracing::warn!(%error, "presence beat not written");
        }
    }
}

/// What a scheduled beat says under `status`: nothing while it hides
/// presence.
#[must_use]
pub fn scheduled_state(status: SessionStatus) -> Option<BeatState> {
    (!crate::hides_presence(status)).then(|| status.into())
}

#[cfg(test)]
#[path = "beat_tests.rs"]
mod tests;
