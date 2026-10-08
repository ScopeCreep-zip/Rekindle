use std::cmp::Ordering;
use std::fmt;
use std::ops::{Add, AddAssign, Div, Neg as _, Sub, SubAssign};
use std::time::{Duration, Instant};

use crate::Bitrate;
use crate::DataSize;

/// Wrapper for [`Instant`] that provides additional time points in the past or future.
///
/// str0m names this `Timestamp`; it is renamed here because the workspace's
/// duplicate-type gate already has a `Timestamp` (in `rekindle-ipc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BweTimestamp {
    /// A time in the past that already happened.
    DistantPast,

    /// An exact instant.
    Exact(Instant),

    /// A time in the future that will never happen.
    DistantFuture,
}

/// Wrapper for [`Duration`] that can be negative and provides a duration to a
/// distant future or past.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TimeDelta {
    /// Time delta to some event in distant past that already happened.
    NegativeInfinity,

    /// An exact negative duration.
    Negative(Duration),

    /// An exact positive duration.
    Positive(Duration),

    /// Time delta to some event in distant future that will never happen.
    PositiveInfinity,
}

impl TimeDelta {
    pub(super) const ZERO: Self = Self::Positive(Duration::ZERO);

    /// Returns the number of seconds contained by this [`TimeDelta`] as `f64`.
    pub fn as_secs_f64(&self) -> f64 {
        match self {
            Self::NegativeInfinity => f64::NEG_INFINITY,
            Self::Negative(d) => d.as_secs_f64().neg(),
            Self::Positive(d) => d.as_secs_f64(),
            Self::PositiveInfinity => f64::INFINITY,
        }
    }
}

#[cfg(test)]
impl TimeDelta {
    /// Creates a [`TimeDelta`] from seconds.
    pub const fn from_secs(secs: i64) -> TimeDelta {
        if secs >= 0 {
            Self::Positive(Duration::from_secs(secs.unsigned_abs()))
        } else {
            Self::Negative(Duration::from_secs(secs.unsigned_abs()))
        }
    }

    /// Creates a [`TimeDelta`] from milliseconds.
    pub const fn from_millis(millis: i64) -> Self {
        if millis >= 0 {
            Self::Positive(Duration::from_millis(millis.unsigned_abs()))
        } else {
            Self::Negative(Duration::from_millis(millis.unsigned_abs()))
        }
    }
}

impl BweTimestamp {
    /// Indicates whether this [`BweTimestamp`] is [`BweTimestamp::Exact`].
    pub const fn is_exact(&self) -> bool {
        matches!(self, Self::Exact(_))
    }
}

impl Add<TimeDelta> for BweTimestamp {
    type Output = Self;

    fn add(self, rhs: TimeDelta) -> Self::Output {
        match (self, rhs) {
            (Self::DistantFuture, _) | (_, TimeDelta::PositiveInfinity) => Self::DistantFuture,
            (Self::DistantPast, _) | (_, TimeDelta::NegativeInfinity) => Self::DistantPast,
            (Self::Exact(i), TimeDelta::Negative(d)) => Self::Exact(i.checked_sub(d).unwrap()),
            (Self::Exact(i), TimeDelta::Positive(d)) => Self::Exact(i + d),
        }
    }
}

impl Sub<TimeDelta> for BweTimestamp {
    type Output = Self;

    fn sub(self, rhs: TimeDelta) -> Self::Output {
        match (self, rhs) {
            (Self::DistantFuture, _) | (_, TimeDelta::NegativeInfinity) => Self::DistantFuture,
            (Self::DistantPast, _) | (_, TimeDelta::PositiveInfinity) => Self::DistantPast,
            (Self::Exact(i), TimeDelta::Negative(d)) => Self::Exact(i.checked_add(d).unwrap()),
            (Self::Exact(i), TimeDelta::Positive(d)) => Self::Exact(i.checked_sub(d).unwrap()),
        }
    }
}

impl Sub<Self> for BweTimestamp {
    type Output = TimeDelta;

    fn sub(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::DistantFuture, _) | (_, Self::DistantPast) => TimeDelta::PositiveInfinity,
            (Self::DistantPast, _) | (_, Self::DistantFuture) => TimeDelta::NegativeInfinity,
            (Self::Exact(this), Self::Exact(that)) => match this.cmp(&that) {
                Ordering::Less => TimeDelta::Negative(that - this),
                Ordering::Equal => TimeDelta::ZERO,
                Ordering::Greater => TimeDelta::Positive(this - that),
            },
        }
    }
}

impl Add<Duration> for BweTimestamp {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self::Output {
        self + TimeDelta::from(rhs)
    }
}

impl Sub<Duration> for BweTimestamp {
    type Output = Self;

    fn sub(self, rhs: Duration) -> Self::Output {
        self - TimeDelta::from(rhs)
    }
}

impl Sub<Instant> for BweTimestamp {
    type Output = TimeDelta;

    fn sub(self, rhs: Instant) -> Self::Output {
        self.sub(Self::from(rhs))
    }
}

impl SubAssign<TimeDelta> for BweTimestamp {
    fn sub_assign(&mut self, rhs: TimeDelta) {
        *self = *self - rhs;
    }
}

impl AddAssign<TimeDelta> for BweTimestamp {
    fn add_assign(&mut self, rhs: TimeDelta) {
        *self = *self + rhs;
    }
}

impl SubAssign<Duration> for BweTimestamp {
    fn sub_assign(&mut self, rhs: Duration) {
        *self = *self - rhs;
    }
}

impl AddAssign<Duration> for BweTimestamp {
    fn add_assign(&mut self, rhs: Duration) {
        *self = *self + rhs;
    }
}

impl PartialOrd for BweTimestamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(Self::cmp(self, other))
    }
}

impl Ord for BweTimestamp {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::DistantPast, Self::DistantPast) | (Self::DistantFuture, Self::DistantFuture) => {
                Ordering::Equal
            }
            (_, Self::DistantPast) | (Self::DistantFuture, _) => Ordering::Greater,
            (Self::DistantPast, _) | (_, Self::DistantFuture) => Ordering::Less,
            (Self::Exact(v1), Self::Exact(v2)) => v1.cmp(v2),
        }
    }
}

impl From<Instant> for BweTimestamp {
    fn from(value: Instant) -> Self {
        Self::Exact(value)
    }
}

impl Add<Self> for TimeDelta {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::PositiveInfinity, _) | (_, Self::PositiveInfinity) => Self::PositiveInfinity,
            (Self::NegativeInfinity, _) | (_, Self::NegativeInfinity) => Self::NegativeInfinity,
            (Self::Negative(this), Self::Negative(that)) => Self::Negative(this + that),
            (Self::Positive(this), Self::Positive(that)) => Self::Positive(this + that),
            (Self::Positive(this), Self::Negative(that)) => match this.cmp(&that) {
                Ordering::Less => Self::Negative(that.checked_sub(this).unwrap()),
                Ordering::Equal => Self::ZERO,
                Ordering::Greater => Self::Positive(this.checked_sub(that).unwrap()),
            },
            (Self::Negative(this), Self::Positive(that)) => match this.cmp(&that) {
                Ordering::Less => Self::Positive(that.checked_sub(this).unwrap()),
                Ordering::Equal => Self::ZERO,
                Ordering::Greater => Self::Negative(this.checked_sub(that).unwrap()),
            },
        }
    }
}

impl Sub<Self> for TimeDelta {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::PositiveInfinity, _) | (_, Self::NegativeInfinity) => Self::PositiveInfinity,
            (Self::NegativeInfinity, _) | (_, Self::PositiveInfinity) => Self::NegativeInfinity,
            (Self::Positive(this), Self::Negative(that)) => Self::Positive(this + that),
            (Self::Negative(this), Self::Positive(that)) => Self::Negative(this + that),
            (Self::Positive(this), Self::Positive(that)) => match this.cmp(&that) {
                Ordering::Less => Self::Negative(that.checked_sub(this).unwrap()),
                Ordering::Equal => Self::ZERO,
                Ordering::Greater => Self::Positive(this.checked_sub(that).unwrap()),
            },
            (Self::Negative(this), Self::Negative(that)) => match this.cmp(&that) {
                Ordering::Less => Self::Positive(that.checked_sub(this).unwrap()),
                Ordering::Equal => Self::ZERO,
                Ordering::Greater => Self::Negative(this.checked_sub(that).unwrap()),
            },
        }
    }
}

impl PartialOrd for TimeDelta {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(Self::cmp(self, other))
    }
}

impl Ord for TimeDelta {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::NegativeInfinity, Self::NegativeInfinity)
            | (Self::PositiveInfinity, Self::PositiveInfinity) => Ordering::Equal,
            (_, Self::NegativeInfinity)
            | (Self::PositiveInfinity, _)
            | (Self::Positive(_), Self::Negative(_)) => Ordering::Greater,
            (Self::NegativeInfinity, _)
            | (_, Self::PositiveInfinity)
            | (Self::Negative(_), Self::Positive(_)) => Ordering::Less,
            (Self::Positive(this), Self::Positive(that)) => this.cmp(that),
            (Self::Negative(this), Self::Negative(that)) => that.cmp(this),
        }
    }
}

impl PartialEq<Duration> for TimeDelta {
    fn eq(&self, other: &Duration) -> bool {
        *self == Self::from(*other)
    }
}

impl PartialOrd<Duration> for TimeDelta {
    fn partial_cmp(&self, other: &Duration) -> Option<Ordering> {
        Some(Self::cmp(self, &Self::from(*other)))
    }
}

impl SubAssign<Self> for TimeDelta {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl AddAssign<Self> for TimeDelta {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Div<u32> for TimeDelta {
    type Output = Self;

    #[inline]
    fn div(self, rhs: u32) -> Self {
        match self {
            Self::NegativeInfinity | Self::PositiveInfinity => self,
            Self::Negative(duration) => Self::Negative(duration / rhs),
            Self::Positive(duration) => Self::Positive(duration / rhs),
        }
    }
}

impl From<Duration> for TimeDelta {
    fn from(value: Duration) -> Self {
        Self::Positive(value)
    }
}

impl Div<TimeDelta> for DataSize {
    type Output = Bitrate;

    fn div(self, rhs: TimeDelta) -> Self::Output {
        let bytes = self.as_bytes_f64();
        let s = rhs.as_secs_f64();

        if s == 0.0 {
            return Bitrate::ZERO;
        }

        let bps = (bytes * 8.0) / s;

        bps.into()
    }
}

impl fmt::Display for TimeDelta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimeDelta::NegativeInfinity => write!(f, "-Inf"),
            TimeDelta::Negative(v) => write!(f, "-{:.03}", v.as_secs_f32()),
            TimeDelta::Positive(v) => write!(f, "{:.03}", v.as_secs_f32()),
            TimeDelta::PositiveInfinity => write!(f, "+Inf"),
        }
    }
}

#[cfg(test)]
mod test;
