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
}
