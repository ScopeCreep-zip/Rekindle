//! Monotonic clearance check for content-addressed dedup.
//!
//! An entry stored at clearance C may only be referenced by a sender
//! whose current clearance is >= C.

use crate::v3::wire::clearance::Clearance;

/// Returns true if `sender_clearance >= entry_clearance`.
pub fn check_clearance(entry_clearance: Clearance, sender_clearance: Clearance) -> bool {
    sender_clearance >= entry_clearance
}
