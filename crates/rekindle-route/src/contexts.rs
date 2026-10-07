//! Routing-context selection metadata for application traffic.
//!
//! Every path uses a Veilid Safe route at the uniform
//! [`rekindle_types::config::ANONYMITY_HOP_FLOOR`] (3-hop Tor-class).
//! There is no Safe-vs-Unsafe split and no per-class hop distinction —
//! only `ordered` (sequencing preference) differs at this layer.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteContextSpec {
    /// Safety-route relay hops. Always the anonymity floor.
    pub hop_count: usize,
    /// Prefer ordered delivery on the safety route.
    pub ordered: bool,
}

impl RouteContextSpec {
    /// This spec as a [`SafetyProfile`](rekindle_types::config::SafetyProfile),
    /// for the one profile-to-Veilid mapping
    /// (`rekindle_protocol::dht::pool::safety_selection`).
    #[must_use]
    pub fn safety_profile(&self) -> rekindle_types::config::SafetyProfile {
        use rekindle_types::config::{
            SafetyProfile, SequencingPreference, StabilityPreference, ANONYMITY_HOP_FLOOR,
        };
        SafetyProfile {
            hop_count: u8::try_from(self.hop_count).unwrap_or(ANONYMITY_HOP_FLOOR),
            stability: StabilityPreference::Reliable,
            sequencing: if self.ordered {
                SequencingPreference::PreferOrdered
            } else {
                SequencingPreference::NoPreference
            },
        }
    }

    /// The single anonymous routing spec used for every application path.
    pub fn rc_safe() -> Self {
        Self {
            hop_count: rekindle_types::config::ANONYMITY_HOP_FLOOR as usize,
            ordered: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RouteContextSpec;

    #[test]
    fn safe_spec_matches_anonymity_floor() {
        let safe = RouteContextSpec::rc_safe();
        assert_eq!(
            safe.hop_count,
            rekindle_types::config::ANONYMITY_HOP_FLOOR as usize
        );
        assert!(!safe.ordered);
    }

    /// The desktop's send context: what it hand-built before the one
    /// mapping took over (floor hops, Reliable, unordered).
    #[test]
    fn safe_spec_profile_is_floor_reliable_unordered() {
        use rekindle_types::config::{
            SequencingPreference, StabilityPreference, ANONYMITY_HOP_FLOOR,
        };
        let profile = RouteContextSpec::rc_safe().safety_profile();
        assert_eq!(profile.hop_count, ANONYMITY_HOP_FLOOR);
        assert_eq!(profile.stability, StabilityPreference::Reliable);
        assert_eq!(profile.sequencing, SequencingPreference::NoPreference);
    }
}
