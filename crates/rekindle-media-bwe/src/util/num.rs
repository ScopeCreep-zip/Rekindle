//! Numeric conversions with exactly the semantics of Rust's `as` casts.
//!
//! str0m converts between integers and `f64` with `as`. The workspace lints
//! reject those casts (`clippy::cast_*`) and forbid lint suppressions, so each
//! cast str0m makes goes through one of these functions instead. Every function
//! returns, for every input, the value the corresponding `as` cast returns:
//! int→float rounds to nearest, ties to even; float→int truncates toward zero,
//! saturates at the bounds and maps NaN to 0.

/// 2^32 as `f64`.
const TWO_POW_32: f64 = 4_294_967_296.0;
/// 2^63 as `f64`.
const TWO_POW_63: f64 = 9_223_372_036_854_775_808.0;
/// 2^64 as `f64`.
const TWO_POW_64: f64 = 18_446_744_073_709_551_616.0;
/// Width of the `f64` mantissa field.
const MANTISSA_BITS: u64 = 52;
/// Exponent bias of `f64`.
const EXPONENT_BIAS: u64 = 1023;

/// `v as f64`.
///
/// Both halves convert exactly, and the final addition rounds the exact sum
/// once, to nearest even: the same rounding `as` performs.
pub(crate) const fn u64_to_f64(v: u64) -> f64 {
    let hi = (v >> 32) as u32;
    let lo = (v & 0xffff_ffff) as u32;
    hi as f64 * TWO_POW_32 + lo as f64
}

/// `v as f64`, by the same argument as [`u64_to_f64`].
pub(crate) fn i64_to_f64(v: i64) -> f64 {
    let hi = i32::try_from(v >> 32).unwrap_or_default();
    let lo = u32::try_from(v & i64::from(u32::MAX)).unwrap_or_default();
    f64::from(hi) * TWO_POW_32 + f64::from(lo)
}

/// `v as f64`.
pub(crate) fn usize_to_f64(v: usize) -> f64 {
    // usize is at most 64 bits on every supported target.
    u64_to_f64(u64::try_from(v).unwrap_or(u64::MAX))
}

/// `v as i64`: the same 64 bits, reinterpreted (wraps above `i64::MAX`).
pub(crate) const fn u64_as_i64(v: u64) -> i64 {
    i64::from_ne_bytes(v.to_ne_bytes())
}

/// `v as i64` for `usize` (at most 64 bits on every supported target).
pub(crate) fn usize_as_i64(v: usize) -> i64 {
    u64_as_i64(u64::try_from(v).unwrap_or(u64::MAX))
}

/// `v as u32` for `usize`: the low 32 bits.
pub(crate) fn usize_as_u32(v: usize) -> u32 {
    let v = u64::try_from(v).unwrap_or(u64::MAX);
    u32::try_from(v & u64::from(u32::MAX)).unwrap_or_default()
}

/// `v as i32` for `usize`: the low 32 bits, reinterpreted.
pub(crate) fn usize_as_i32(v: usize) -> i32 {
    i32::from_ne_bytes(usize_as_u32(v).to_ne_bytes())
}

/// `v as usize` for `u64`: the low bits that fit a `usize`.
pub(crate) fn u64_as_usize(v: u64) -> usize {
    let mask = u64::try_from(usize::MAX).unwrap_or(u64::MAX);
    usize::try_from(v & mask).unwrap_or_default()
}

/// `v as u64` for `u128`: the low 64 bits.
pub(crate) fn u128_as_u64(v: u128) -> u64 {
    u64::try_from(v & u128::from(u64::MAX)).unwrap_or_default()
}

/// `v as f64` for `u128`.
///
/// Values above 64 bits are shifted down to 64 bits with a sticky bit (the OR
/// of every bit shifted out), converted, then scaled back by a power of two.
/// The sticky bit sits well below the 53-bit rounding position, so the single
/// rounding in [`u64_to_f64`] is the correct rounding of `v`; the scaling is
/// exact.
pub(crate) fn u128_to_f64(v: u128) -> f64 {
    let Ok(narrow) = u64::try_from(v) else {
        let shift = 64 - v.leading_zeros();
        let sticky = u128::from(v & ((1u128 << shift) - 1) != 0);
        let reduced = u128_as_u64((v >> shift) | sticky);
        let scale = i32::try_from(shift).unwrap_or_default();
        return u64_to_f64(reduced) * 2f64.powi(scale);
    };
    u64_to_f64(narrow)
}

/// `v as u64`: truncate toward zero, saturate, NaN → 0.
pub(crate) fn f64_to_u64(v: f64) -> u64 {
    if v.is_nan() || v <= 0.0 {
        return 0;
    }
    if v >= TWO_POW_64 {
        return u64::MAX;
    }
    // 0 < v < 2^64: read the integer part off the IEEE 754 bit pattern.
    let bits = v.trunc().to_bits();
    let exponent = (bits >> MANTISSA_BITS) & 0x7ff;
    if exponent < EXPONENT_BIAS {
        // |v| < 1
        return 0;
    }
    let mantissa = (bits & ((1 << MANTISSA_BITS) - 1)) | (1 << MANTISSA_BITS);
    let shift = exponent - EXPONENT_BIAS;
    if shift >= MANTISSA_BITS {
        mantissa << (shift - MANTISSA_BITS)
    } else {
        mantissa >> (MANTISSA_BITS - shift)
    }
}

/// `v as i64`: truncate toward zero, saturate, NaN → 0.
pub(crate) fn f64_to_i64(v: f64) -> i64 {
    if v.is_nan() {
        return 0;
    }
    if v >= TWO_POW_63 {
        return i64::MAX;
    }
    if v <= -TWO_POW_63 {
        return i64::MIN;
    }
    // |v| < 2^63, so the magnitude fits.
    let magnitude = i64::try_from(f64_to_u64(v.abs())).unwrap_or(i64::MAX);
    if v < 0.0 {
        -magnitude
    } else {
        magnitude
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Bit-for-bit `f64` equality (what `as` produces is exact, not approximate).
    fn assert_same(actual: f64, expected: f64) {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "{actual} != {expected}"
        );
    }

    #[test]
    fn int_to_float() {
        assert_same(u64_to_f64(0), 0.0);
        assert_same(u64_to_f64(1_234_567), 1_234_567.0);
        assert_same(u64_to_f64(u64::MAX), TWO_POW_64);
        // 2^53 + 1 is not representable; ties go to even (2^53).
        assert_same(u64_to_f64((1 << 53) + 1), 9_007_199_254_740_992.0);
        // 2^53 + 3 rounds up to 2^53 + 4.
        assert_same(u64_to_f64((1 << 53) + 3), 9_007_199_254_740_996.0);
        assert_same(i64_to_f64(-1), -1.0);
        assert_same(i64_to_f64(-1_234_567_890_123), -1_234_567_890_123.0);
        assert_same(i64_to_f64(i64::MIN), -TWO_POW_63);
        assert_same(i64_to_f64(i64::MAX), TWO_POW_63);
        assert_same(usize_to_f64(42), 42.0);
        assert_same(u128_to_f64(1 << 100), 2f64.powi(100));
        // 2^100 + 2^47 lies exactly between two f64s (spacing 2^48): ties to even.
        assert_same(u128_to_f64((1 << 100) + (1 << 47)), 2f64.powi(100));
        // One more unit is past the tie and rounds up.
        assert_same(
            u128_to_f64((1 << 100) + (1 << 47) + 1),
            2f64.powi(100) + 2f64.powi(48),
        );
        assert_same(u128_to_f64(12_345), 12_345.0);
    }

    #[test]
    fn wrapping_int_casts() {
        assert_eq!(u64_as_i64(u64::MAX), -1);
        assert_eq!(usize_as_i64(7), 7);
        assert_eq!(usize_as_u32(0x1_0000_0005), 5);
        assert_eq!(usize_as_i32(0xffff_ffff), -1);
        assert_eq!(u64_as_usize(9), 9);
        assert_eq!(u128_as_u64((1 << 64) + 3), 3);
    }

    #[test]
    fn float_to_int() {
        assert_eq!(f64_to_u64(f64::NAN), 0);
        assert_eq!(f64_to_u64(-3.7), 0);
        assert_eq!(f64_to_u64(0.999), 0);
        assert_eq!(f64_to_u64(1.0), 1);
        assert_eq!(f64_to_u64(3.7), 3);
        assert_eq!(f64_to_u64(1e300), u64::MAX);
        assert_eq!(f64_to_u64(f64::INFINITY), u64::MAX);
        assert_eq!(f64_to_u64(9_007_199_254_740_993.0), 9_007_199_254_740_992);
        assert_eq!(
            f64_to_u64(18_446_744_073_709_549_568.0),
            18_446_744_073_709_549_568
        );
        assert_eq!(f64_to_i64(f64::NAN), 0);
        assert_eq!(f64_to_i64(-3.7), -3);
        assert_eq!(f64_to_i64(-0.5), 0);
        assert_eq!(f64_to_i64(2.5), 2);
        assert_eq!(f64_to_i64(f64::NEG_INFINITY), i64::MIN);
        assert_eq!(f64_to_i64(f64::INFINITY), i64::MAX);
        assert_eq!(f64_to_i64(-TWO_POW_63), i64::MIN);
    }
}
