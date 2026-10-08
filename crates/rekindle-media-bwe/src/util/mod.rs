//! Helpers the estimator and pacer import from str0m's `src/util`.

use std::time::Instant;

mod average;
mod num;
mod time_tricks;

pub(crate) use average::MovingAverage;
pub(crate) use num::{
    f64_to_i64, f64_to_u64, i64_to_f64, u128_as_u64, u128_to_f64, u64_as_i64, u64_as_usize,
    u64_to_f64, usize_as_i32, usize_as_i64, usize_as_u32, usize_to_f64,
};
pub(crate) use time_tricks::{already_happened, not_happening};

/// Pick the soonest of two `(deadline, reason)` pairs.
pub(crate) trait Soonest {
    fn soonest(self, other: Self) -> Self;
}

impl<T: Default> Soonest for (Option<Instant>, T) {
    fn soonest(self, other: Self) -> Self {
        match (self, other) {
            ((Some(v1), s1), (Some(v2), s2)) => {
                if v1 < v2 {
                    (Some(v1), s1)
                } else {
                    (Some(v2), s2)
                }
            }
            ((None, _), (None, _)) => (None, T::default()),
            ((None, _), (v, s)) | ((v, s), (None, _)) => (v, s),
        }
    }
}
