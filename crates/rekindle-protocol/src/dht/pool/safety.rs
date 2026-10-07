//! The one mapping from a data class's safety profile to a Veilid safety
//! selection (V19).

use rekindle_types::config::{
    SafetyProfile, SequencingPreference, StabilityPreference, ANONYMITY_HOP_FLOOR,
};
use veilid_core::{SafetySelection, SafetySpec, Sequencing, Stability};

/// The one mapping from a data class's profile to a Veilid safety
/// selection. Always Safe, never below [`ANONYMITY_HOP_FLOOR`].
pub fn safety_selection(profile: &SafetyProfile) -> SafetySelection {
    SafetySelection::Safe(SafetySpec {
        preferred_route: None,
        hop_count: usize::from(profile.hop_count.max(ANONYMITY_HOP_FLOOR)),
        stability: match profile.stability {
            StabilityPreference::LowLatency => Stability::LowLatency,
            StabilityPreference::Reliable => Stability::Reliable,
        },
        sequencing: match profile.sequencing {
            SequencingPreference::NoPreference => Sequencing::PreferUnordered,
            SequencingPreference::PreferOrdered => Sequencing::PreferOrdered,
            SequencingPreference::EnsureOrdered => Sequencing::EnsureOrdered,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_safety_selection_never_drops_below_the_anonymity_floor() {
        let mut profile = SafetyProfile::default_dht();
        profile.hop_count = 1;
        let SafetySelection::Safe(spec) = safety_selection(&profile) else {
            panic!("always Safe");
        };
        assert_eq!(spec.hop_count, usize::from(ANONYMITY_HOP_FLOOR));
        profile.hop_count = 5;
        profile.stability = StabilityPreference::LowLatency;
        profile.sequencing = SequencingPreference::NoPreference;
        let SafetySelection::Safe(spec) = safety_selection(&profile) else {
            panic!("always Safe");
        };
        assert_eq!(spec.hop_count, 5);
        assert_eq!(spec.stability, Stability::LowLatency);
        assert_eq!(spec.sequencing, Sequencing::PreferUnordered);
    }
}
