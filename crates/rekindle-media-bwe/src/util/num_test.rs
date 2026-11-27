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
