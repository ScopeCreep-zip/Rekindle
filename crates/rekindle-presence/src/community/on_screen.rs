//! The members on screen (plan E3.4 Q1, decided 2026-11-19): the 15-minute
//! liveness rule covers the ring plus the members a frontend shows, at
//! most [`MEMBERS_ON_SCREEN_MAX`] (Discord's `large_threshold` floor of
//! 50); every other member keeps its last-known status. A frontend reports
//! which members it shows; the Host inspects the beats of those outside
//! the ring once per beat period. Those records are deliberately unwatched
//! (the ring holds the watch budget), so the inspection is the one app
//! inspect D4 allows ("App inspect runs only on deliberately unwatched
//! records"), on a set an event defines (the frontend's report), never a
//! poll of a watched record. Veilid-free; the Host holds one per community.

use std::collections::BTreeMap;

use rekindle_codec::member_record::BEAT_PERIOD_MS;

/// The most members a frontend may report on screen; a longer report is
/// refused (validation), never cut.
pub const MEMBERS_ON_SCREEN_MAX: usize = 50;

/// How often an on-screen member's beat is inspected: once per beat
/// period, the writer's mean cadence (plan E3.4 "Cadence").
pub const ON_SCREEN_INSPECT_MS: u64 = BEAT_PERIOD_MS;

/// One community's members on screen (pseudonym hex) and when each was
/// last inspected (our clock, ms).
#[derive(Debug, Clone, Default)]
pub struct OnScreen {
    members: BTreeMap<String, Option<u64>>,
}

impl OnScreen {
    /// Replace the set with `members` (at most [`MEMBERS_ON_SCREEN_MAX`],
    /// checked by the caller). A member kept keeps its last inspection; a
    /// member gone forgets it. The members new on screen, to inspect at
    /// once.
    pub fn set(&mut self, members: Vec<String>) -> Vec<String> {
        let mut next = BTreeMap::new();
        let mut added = Vec::new();
        for member in members {
            if next.contains_key(&member) {
                continue;
            }
            let held = self.members.get(&member).copied();
            if held.is_none() {
                added.push(member.clone());
            }
            next.insert(member, held.flatten());
        }
        self.members = next;
        added
    }

    /// The members on screen.
    pub fn members(&self) -> impl Iterator<Item = &String> {
        self.members.keys()
    }

    /// Record that `member` is inspected at `now_ms`; the arrival to give
    /// the beat that inspection reads (plan E3.4 Q4): the previous
    /// inspection's time, since a beat newer than the one held then was
    /// written after it (the earliest it can have arrived), or `None` for
    /// the first inspection, a cold read in which only the beat number
    /// counts. `None` too for a member not on screen.
    pub fn inspected(&mut self, member: &str, now_ms: u64) -> Option<u64> {
        self.members
            .get_mut(member)
            .and_then(|last| last.replace(now_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("{i:064x}")).collect()
    }

    #[test]
    fn a_first_inspection_is_cold_and_a_later_one_arrives_after_the_previous() {
        let mut on_screen = OnScreen::default();
        assert_eq!(on_screen.set(keys(2)), keys(2));
        assert_eq!(on_screen.inspected(&keys(1)[0], 1_000), None);
        assert_eq!(on_screen.inspected(&keys(1)[0], 301_000), Some(1_000));
        assert_eq!(on_screen.inspected("not on screen", 5), None);
    }

    #[test]
    fn a_member_kept_keeps_its_inspection_and_one_gone_forgets_it() {
        let mut on_screen = OnScreen::default();
        on_screen.set(keys(2));
        on_screen.inspected(&keys(2)[0], 10);
        on_screen.inspected(&keys(2)[1], 10);
        // Member 1 scrolls away; member 0 stays; member 2 is new.
        let mut next = keys(3);
        next.remove(1);
        assert_eq!(on_screen.set(next), vec![keys(3)[2].clone()]);
        assert_eq!(on_screen.inspected(&keys(2)[0], 20), Some(10));
        // Back on screen: a cold read again.
        on_screen.set(keys(2));
        assert_eq!(on_screen.inspected(&keys(2)[1], 30), None);
        assert_eq!(on_screen.members().count(), 2);
    }
}
