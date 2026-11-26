use super::*;
use veilid_core::{SequenceOrdering, Sequencing};

/// Plan VU decision 2: Media is Unordered (UDP), at the configured
/// default hop count; a custom route carries only its preference's
/// ordering (`route_allocate.rs:936`).
#[test]
fn media_is_unordered_at_the_default_hop_count() {
    let media = media_private_spec();
    assert_eq!(media.hop_count, 0, "0 = the configured default");
    assert_eq!(media.sequencing, Sequencing::PreferUnordered);
    assert_eq!(
        media.sequencing.preferred_ordering(),
        SequenceOrdering::Unordered
    );
}
