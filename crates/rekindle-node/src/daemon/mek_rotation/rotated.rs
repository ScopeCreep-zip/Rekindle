//! Departures already rotated for (A/B finding R4-1, plan E3.5).
//!
//! One removal reaches the worker more than once: the remover queues the
//! departure when it writes the `MemberRemoved`, then the `Kick` notice it
//! gossiped comes back to it relayed by the mesh, and every other member
//! hears the notice once per relay that delivers it. Each copy elects a
//! rotator from the generation it starts at, so a copy that arrives after
//! the first rotation landed would mint the next generation for a member
//! who no longer holds any key: two rotations for one kick.
//!
//! The record is keyed on the departure itself, the community and the
//! departed member's membership as the first copy saw it (its
//! `MemberJoined` stamp while still listed, none once the removal merged).
//! A repeat is spent once the key has moved past the generation the first
//! copy started at, by our rotation or a better-ranked peer's. It is not
//! spent while no rotation has landed (nobody eligible was online, so the
//! departure is still owed), nor after the member was admitted again
//! under a newer `MemberJoined`: a re-admitted member holds the current
//! key, and its next removal is a new departure.

use std::collections::HashMap;

use rekindle_governance::op::OpStamp;

/// The departed member's membership as one copy of the departure sees it:
/// the stamp of its `MemberJoined` while it is listed.
pub type Membership = Option<OpStamp>;

#[derive(Debug, Clone, Copy)]
struct Rotated {
    /// The key generation the first copy started its rotation at.
    from_generation: u64,
    /// The membership that copy saw.
    membership: Membership,
}

/// The worker's record of departures it already acted on.
#[derive(Debug, Default)]
pub struct RotatedDepartures(HashMap<(String, String), Rotated>);

impl RotatedDepartures {
    /// Whether a copy of the departure of `departed` from `community`,
    /// seen with `membership` while the key is at `generation`, repeats a
    /// departure that was already rotated for.
    #[must_use]
    pub fn is_spent(
        &self,
        community: &str,
        departed: &str,
        membership: Membership,
        generation: u64,
    ) -> bool {
        self.0
            .get(&key(community, departed))
            .is_some_and(|rotated| {
                let readmitted = membership.is_some() && membership != rotated.membership;
                generation > rotated.from_generation && !readmitted
            })
    }

    /// Record that a rotation for this departure ran from `generation`.
    pub fn record(
        &mut self,
        community: &str,
        departed: &str,
        membership: Membership,
        generation: u64,
    ) {
        self.0.insert(
            key(community, departed),
            Rotated {
                from_generation: generation,
                membership,
            },
        );
    }
}

fn key(community: &str, departed: &str) -> (String, String) {
    (community.to_owned(), departed.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::governance::OpHash;

    fn joined(lamport: u64) -> OpStamp {
        OpStamp {
            lamport,
            hash: OpHash([7; 32]),
        }
    }

    #[test]
    fn a_copy_after_the_rotation_landed_is_spent() {
        let mut rotated = RotatedDepartures::default();
        rotated.record("c", "AB", None, 1);
        assert!(rotated.is_spent("c", "ab", None, 2));
        assert!(!rotated.is_spent("other", "ab", None, 2));
    }

    #[test]
    fn a_copy_seen_before_the_removal_merged_matches_one_seen_after() {
        let mut rotated = RotatedDepartures::default();
        rotated.record("c", "ab", Some(joined(3)), 1);
        assert!(rotated.is_spent("c", "ab", None, 2));
        assert!(rotated.is_spent("c", "ab", Some(joined(3)), 2));
    }

    #[test]
    fn a_departure_nobody_rotated_for_is_still_owed() {
        let mut rotated = RotatedDepartures::default();
        rotated.record("c", "ab", None, 1);
        assert!(!rotated.is_spent("c", "ab", None, 1));
    }

    #[test]
    fn a_readmitted_member_departs_anew() {
        let mut rotated = RotatedDepartures::default();
        rotated.record("c", "ab", None, 1);
        assert!(!rotated.is_spent("c", "ab", Some(joined(9)), 2));
        rotated.record("c", "ab", Some(joined(3)), 1);
        assert!(!rotated.is_spent("c", "ab", Some(joined(9)), 2));
    }
}
