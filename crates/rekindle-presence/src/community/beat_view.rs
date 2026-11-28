//! The beat view (plan E3.4 S3, "Staleness" and "Watching"): what each
//! member's beat (`M`/1) shows of them in one community, from the beats a
//! ring watch, a `HeadAdvanced` hint or a read delivered, and their local
//! expiry. Veilid-free; the Host's presence adapter (`rekindle-node`
//! `host/adapters/presence/`) feeds it and publishes what it shows, on
//! both modes.
//!
//! - **Shown status.** A member shows its beat's state while the beat is
//!   present (online, away, busy) and live ([`beat_is_live`]), and offline
//!   otherwise: an `Offline` or `Departed` beat, or a live one gone stale.
//! - **Expiry is local.** A member whose last beat goes stale turns offline
//!   here, with no new write and no `BeatExpired` message
//!   ([`BeatView::expire`]); the caller wakes at [`BeatView::next_expiry`].
//! - **Ordering.** A beat carries its writer's persisted, increasing `seq`
//!   (`communities.beat_seq`); a beat at or below the seq held is an older
//!   copy (a lagging replica or a repeated hint) and changes nothing.
//! - **Voice** (plan E3.4 S8-pre): a member present and live claims its
//!   beat's `voiceChannel`; [`BeatView::voice_rows`] is what the voice
//!   roster reconcile is handed on each change and expiry.

use std::collections::BTreeMap;

use rekindle_codec::member_record::{BeatState, PresenceBeat, SealedExtras, BEAT_PERIOD_MS};

use crate::deps::VoicePresenceRow;

/// How long a beat shows its member live: 3 × the 5-minute mean beat
/// period (plan E3.4 "Staleness"; Matrix Synapse's federation timeout is
/// the same order).
pub const BEAT_LIVENESS_MS: u64 = 3 * BEAT_PERIOD_MS;

/// Until when (our clock, ms) a beat shows its member live (plan E3.4 Q4):
/// [`BEAT_LIVENESS_MS`] from the later of its beat number's time (the
/// start of the writer's 5-minute period) and `arrived_at_ms`, when a watch
/// or hint delivered it to us as a change. A writer whose clock runs behind
/// ours is then still live for 15 minutes after its beat arrives.
///
/// A beat read cold, with nothing telling when it was written (a joiner's
/// bootstrap read, our first read of a member), has no arrival (`None`):
/// only its beat number counts, since the read's own time says nothing of
/// the beat's age.
///
/// A beat number more than one period ([`BEAT_PERIOD_MS`]) ahead of our
/// clock when the beat arrived counts from its arrival instead (plan E3.4
/// Q4 as amended 2026-11-19): a writer's honest clock puts the number at
/// most a period past ours, and a number further ahead would otherwise
/// keep its member live long after the beats stop.
#[must_use]
pub fn live_until(beat_number: u64, arrived_at_ms: Option<u64>) -> u64 {
    let written = beat_number.saturating_mul(BEAT_PERIOD_MS);
    let from = match arrived_at_ms {
        Some(arrived) if written > arrived.saturating_add(BEAT_PERIOD_MS) => arrived,
        Some(arrived) => arrived.max(written),
        None => written,
    };
    from.saturating_add(BEAT_LIVENESS_MS)
}

/// Whether a beat still shows its member live at `now_ms` on our clock
/// ([`live_until`]). The state is not judged here: an `Offline` or
/// `Departed` beat is never live ([`BeatState::is_present`]).
#[must_use]
pub fn beat_is_live(beat_number: u64, arrived_at_ms: Option<u64>, now_ms: u64) -> bool {
    now_ms < live_until(beat_number, arrived_at_ms)
}

/// One member's last beat and what it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    seq: u64,
    state: BeatState,
    /// [`live_until`] of the beat.
    live_until: u64,
    /// What the view shows of the member now: the beat's state, or
    /// `Offline` once it is not present or not live.
    shown: BeatState,
    /// When the beat reached us (our clock, ms).
    seen_at: u64,
    /// The beat's voice channel.
    voice_channel: Option<String>,
    /// The verified beat's extras, still sealed.
    extras: Option<SealedExtras>,
}

impl Entry {
    /// The voice channel the member claims now: its beat's, while it shows
    /// present.
    fn voice_claim(&self) -> Option<&str> {
        self.voice_channel
            .as_deref()
            .filter(|_| self.shown.is_present())
    }
}

/// What a beat changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Observed {
    /// The beat's state differs from the last held (a status transition,
    /// which a ring watcher relays as a `HeadAdvanced` hint, §4.5).
    pub state_changed: bool,
    /// What the member shows now, when it differs from what it showed
    /// (a member never seen showed offline).
    pub shown: Option<BeatState>,
    /// The member's voice claim ([`BeatView::voice_rows`]) changed.
    pub voice_changed: bool,
}

/// The beat view of one community, by member pseudonym (hex).
#[derive(Debug, Clone, Default)]
pub struct BeatView {
    members: BTreeMap<String, Entry>,
}

impl BeatView {
    /// Take `beat`, `member`'s verified beat, delivered at `arrived_at_ms`
    /// (`None` for a cold read), at `now_ms`.
    pub fn observe(
        &mut self,
        member: &str,
        beat: &PresenceBeat,
        arrived_at_ms: Option<u64>,
        now_ms: u64,
    ) -> Observed {
        let held = self.members.get(member).cloned();
        if held.as_ref().is_some_and(|h| beat.seq <= h.seq) {
            return Observed::default();
        }
        let live_until = live_until(beat.beat_number, arrived_at_ms);
        let shown = if beat.state.is_present() && now_ms < live_until {
            beat.state
        } else {
            BeatState::Offline
        };
        let entry = Entry {
            seq: beat.seq,
            state: beat.state,
            live_until,
            shown,
            seen_at: now_ms,
            voice_channel: beat.voice_channel.clone(),
            extras: beat.extras.clone(),
        };
        let voice_changed = held.as_ref().and_then(Entry::voice_claim) != entry.voice_claim();
        self.members.insert(member.to_owned(), entry);
        let was = held.as_ref().map_or(BeatState::Offline, |h| h.shown);
        Observed {
            state_changed: held.map(|h| h.state) != Some(beat.state),
            shown: (was != shown).then_some(shown),
            voice_changed,
        }
    }

    /// The latest verified live beat's sealed extras.
    #[must_use]
    pub fn sealed_extras(&self, member: &str) -> Option<&SealedExtras> {
        self.members
            .get(member)
            .filter(|entry| entry.shown.is_present())
            .and_then(|entry| entry.extras.as_ref())
    }

    /// Every member's voice claim, for the voice roster reconcile: the
    /// beat's channel while the member shows present (`fresh`), none once
    /// its beat is offline, departed or stale, so a ghost expires.
    #[must_use]
    pub fn voice_rows(&self) -> Vec<VoicePresenceRow> {
        (self.members.iter())
            .map(|(member, e)| {
                let claim = e.voice_claim().map(str::to_owned);
                VoicePresenceRow::claim(member.clone(), claim, e.shown.is_present())
            })
            .collect()
    }

    /// What `member` shows now (offline when never seen).
    #[must_use]
    pub fn shown(&self, member: &str) -> BeatState {
        self.members
            .get(member)
            .map_or(BeatState::Offline, |e| e.shown)
    }

    /// When `member`'s last beat reached us (our clock, ms), `None` when
    /// none did.
    #[must_use]
    pub fn seen_at(&self, member: &str) -> Option<u64> {
        self.members.get(member).map(|e| e.seen_at)
    }

    /// Whether `member` may be present: no beat of theirs has reached us
    /// yet (their state is unknown, not offline), or their beat shows them
    /// present. A gossip mesh keeps such a member (plan E3.4 S5): the
    /// fast path must not wait for a joiner's first beat to arrive.
    #[must_use]
    pub fn may_be_present(&self, member: &str) -> bool {
        self.members
            .get(member)
            .is_none_or(|e| e.shown.is_present())
    }

    /// How many members the view shows present (online, away or busy):
    /// the community's online count (Infoview design §2.8, "online member
    /// count, from beats"). A member with no beat yet is not counted.
    #[must_use]
    pub fn present_count(&self) -> usize {
        self.members
            .values()
            .filter(|e| e.shown.is_present())
            .count()
    }

    /// Turn every member whose beat went stale by `now_ms` offline; the
    /// members turned.
    pub fn expire(&mut self, now_ms: u64) -> Vec<String> {
        let mut expired = Vec::new();
        for (member, entry) in &mut self.members {
            if entry.shown.is_present() && now_ms >= entry.live_until {
                entry.shown = BeatState::Offline;
                expired.push(member.clone());
            }
        }
        expired
    }

    /// When the next shown member goes stale (our clock, ms).
    #[must_use]
    pub fn next_expiry(&self) -> Option<u64> {
        (self.members.values())
            .filter(|e| e.shown.is_present())
            .map(|e| e.live_until)
            .min()
    }
}

#[cfg(test)]
#[path = "beat_view_tests.rs"]
mod tests;
