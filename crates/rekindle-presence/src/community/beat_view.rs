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

use rekindle_codec::member_record::{BeatState, PresenceBeat, BEAT_PERIOD_MS};
use rekindle_types::presence::SessionExtras;

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
    /// The beat's extras, opened with our community key and filtered by
    /// our sharing policy (`None` when absent, sealed under a generation we
    /// don't hold, or not shared back).
    extras: Option<SessionExtras>,
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
    members: BTreeMap<String, BTreeMap<String, Entry>>,
}

impl BeatView {
    /// Take a beat from a member with a single record.
    pub fn observe(
        &mut self,
        member: &str,
        beat: &PresenceBeat,
        arrived_at_ms: Option<u64>,
        now_ms: u64,
    ) -> Observed {
        self.observe_record(member, member, beat, arrived_at_ms, now_ms)
    }

    /// Take a verified beat, ordering its sequence within its own record.
    pub fn observe_record(
        &mut self,
        member: &str,
        record: &str,
        beat: &PresenceBeat,
        arrived_at_ms: Option<u64>,
        now_ms: u64,
    ) -> Observed {
        let records = self.members.entry(member.to_owned()).or_default();
        let held = records.get(record);
        if held.is_some_and(|h| beat.seq <= h.seq) {
            return Observed::default();
        }
        let previous = Self::selected(records).cloned();
        let state_changed = held.map(|entry| entry.state) != Some(beat.state);
        let live_until = live_until(beat.beat_number, arrived_at_ms);
        let shown = if beat.state.is_present() && now_ms < live_until {
            beat.state
        } else {
            BeatState::Offline
        };
        records.insert(
            record.to_owned(),
            Entry {
                seq: beat.seq,
                state: beat.state,
                live_until,
                shown,
                seen_at: now_ms,
                voice_channel: beat.voice_channel.clone(),
                extras: None,
            },
        );
        let current = Self::selected(records).expect("the observed record was inserted");
        let was = previous.as_ref().map_or(BeatState::Offline, |e| e.shown);
        Observed {
            state_changed,
            shown: (was != current.shown).then_some(current.shown),
            voice_changed: previous.as_ref().and_then(Entry::voice_claim) != current.voice_claim(),
        }
    }

    fn selected(records: &BTreeMap<String, Entry>) -> Option<&Entry> {
        records
            .values()
            .max_by_key(|e| (e.shown.is_present(), e.seen_at))
    }

    /// Keep extras for a member with a single record.
    pub fn set_extras(&mut self, member: &str, seq: u64, extras: Option<SessionExtras>) {
        self.set_record_extras(member, member, seq, extras);
    }

    /// Keep extras from the same record and beat that is still held.
    pub fn set_record_extras(
        &mut self,
        member: &str,
        record: &str,
        seq: u64,
        extras: Option<SessionExtras>,
    ) {
        if let Some(entry) = self
            .members
            .get_mut(member)
            .and_then(|records| records.get_mut(record))
            .filter(|e| e.seq == seq)
        {
            entry.extras = extras;
        }
    }

    /// The present device's extras, if their beat carried readable ones.
    #[must_use]
    pub fn extras(&self, member: &str) -> Option<&SessionExtras> {
        Self::selected(self.members.get(member)?).and_then(|e| e.extras.as_ref())
    }

    /// Every member's current voice claim.
    #[must_use]
    pub fn voice_rows(&self) -> Vec<VoicePresenceRow> {
        self.members
            .iter()
            .filter_map(|(member, records)| {
                let e = Self::selected(records)?;
                let claim = e.voice_claim().map(str::to_owned);
                Some(VoicePresenceRow::claim(
                    member.clone(),
                    claim,
                    e.shown.is_present(),
                ))
            })
            .collect()
    }

    /// What `member` shows now (offline when never seen).
    #[must_use]
    pub fn shown(&self, member: &str) -> BeatState {
        self.members
            .get(member)
            .and_then(Self::selected)
            .map_or(BeatState::Offline, |e| e.shown)
    }

    /// When the selected device's beat reached us (our clock, ms).
    #[must_use]
    pub fn seen_at(&self, member: &str) -> Option<u64> {
        Self::selected(self.members.get(member)?).map(|e| e.seen_at)
    }

    /// Whether `member` may be present: no beat of theirs has reached us
    /// yet (their state is unknown, not offline), or their beat shows them
    /// present. A gossip mesh keeps such a member (plan E3.4 S5): the
    /// fast path must not wait for a joiner's first beat to arrive.
    #[must_use]
    pub fn may_be_present(&self, member: &str) -> bool {
        self.members
            .get(member)
            .and_then(Self::selected)
            .is_none_or(|e| e.shown.is_present())
    }

    /// How many members the view shows present (online, away or busy):
    /// the community's online count (Infoview design §2.8, "online member
    /// count, from beats"). A member with no beat yet is not counted.
    #[must_use]
    pub fn present_count(&self) -> usize {
        self.members
            .values()
            .filter_map(Self::selected)
            .filter(|e| e.shown.is_present())
            .count()
    }

    /// Drop beats from records that are no longer effective.
    pub fn retain_records(
        &mut self,
        mut keep: impl FnMut(&str, &str) -> bool,
    ) -> Vec<(String, BeatState)> {
        let mut changed = Vec::new();
        self.members.retain(|member, records| {
            let before = records.len();
            records.retain(|record, _| keep(member, record));
            if records.len() != before {
                let shown = Self::selected(records).map_or(BeatState::Offline, |entry| entry.shown);
                changed.push((member.clone(), shown));
            }
            !records.is_empty()
        });
        changed
    }

    /// Turn every member whose beat went stale by `now_ms` offline; the
    /// members turned.
    pub fn expire(&mut self, now_ms: u64) -> Vec<String> {
        let mut expired = Vec::new();
        for (member, records) in &mut self.members {
            let mut changed = false;
            for entry in records.values_mut() {
                if entry.shown.is_present() && now_ms >= entry.live_until {
                    entry.shown = BeatState::Offline;
                    changed = true;
                }
            }
            if changed {
                expired.push(member.clone());
            }
        }
        expired
    }

    /// When the next shown member goes stale (our clock, ms).
    #[must_use]
    pub fn next_expiry(&self) -> Option<u64> {
        self.members
            .values()
            .flat_map(BTreeMap::values)
            .filter(|e| e.shown.is_present())
            .map(|e| e.live_until)
            .min()
    }
}

#[cfg(test)]
#[path = "beat_view_tests.rs"]
mod tests;
