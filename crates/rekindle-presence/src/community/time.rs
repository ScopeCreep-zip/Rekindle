//! Internal time helper for community orchestrators.

#[inline]
pub(super) fn now_secs() -> u64 {
    rekindle_utils::timestamp_secs()
}
