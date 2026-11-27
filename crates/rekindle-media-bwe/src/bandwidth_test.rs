use std::time::Duration;

use super::{Bitrate, DataSize};

#[test]
fn test_bitrate_display() {
    let rate = Bitrate::bps(1);
    assert_eq!(rate.to_string(), "1bit/s");

    let rate = Bitrate::bps(12);
    assert_eq!(rate.to_string(), "12bit/s");

    let rate = Bitrate::bps(123);
    assert_eq!(rate.to_string(), "123bit/s");

    let rate = Bitrate::bps(1234);
    assert_eq!(rate.to_string(), "1.234kbit/s");

    let rate = Bitrate::bps(12345);
    assert_eq!(rate.to_string(), "12.345kbit/s");

    let rate = Bitrate::bps(123_456);
    assert_eq!(rate.to_string(), "123.456kbit/s");

    let rate = Bitrate::bps(1_234_567);
    assert_eq!(rate.to_string(), "1.235Mbit/s");

    let rate = Bitrate::bps(12_345_678);
    assert_eq!(rate.to_string(), "12.346Mbit/s");

    let rate = Bitrate::bps(123_456_789);
    assert_eq!(rate.to_string(), "123.457Mbit/s");

    let rate = Bitrate::bps(1_234_567_898);
    assert_eq!(rate.to_string(), "1.235Gbit/s");

    let rate = Bitrate::bps(12_345_678_987);
    assert_eq!(rate.to_string(), "12.346Gbit/s");

    let rate = Bitrate::bps(123_456_789_876);
    assert_eq!(rate.to_string(), "123.457Gbit/s");

    let rate = Bitrate::bps(1_234_567_898_765);
    assert_eq!(rate.to_string(), "1.235Tbit/s");
}

#[test]
fn test_data_size_div_duration() {
    let size = DataSize::bytes(2_500_000);
    let rate = size / Duration::from_secs(1);

    assert_eq!(rate.as_u64(), 20_000_000);
}

#[test]
fn test_data_size_div_bitrate() {
    let size = DataSize::bytes(12_500);
    let rate = Bitrate::kbps(2_500);
    let duration = size / rate;

    assert_eq!(duration.as_millis(), 40);
}

#[test]
fn test_bitrate_div_f64() {
    let rate = Bitrate::kbps(2_500);
    let new_rate = rate / 2.0;

    assert_eq!(new_rate, Bitrate::kbps(1250));
}

#[test]
fn test_datasize_negative() {
    let size = DataSize::bytes(-100);
    assert_eq!(size.as_bytes_i64(), -100);

    // Test negative kilobytes
    let size_kb = DataSize::kbytes(-5);
    assert_eq!(size_kb.as_bytes_i64(), -5 * 1024);

    // Test that negative values work in comparisons
    let positive = DataSize::bytes(50);
    let negative = DataSize::bytes(-50);
    assert!(negative < positive);
    assert!(negative < DataSize::ZERO);
}

#[test]
fn test_datasize_mul_negative() {
    let size = DataSize::bytes(100);

    // Multiply by -1 to negate
    let negated = size * -1i64;
    assert_eq!(negated, -100);

    // Multiply by -2
    let doubled_neg = size * -2i64;
    assert_eq!(doubled_neg, -200);

    // Negative size multiplied by -1
    let negative_size = DataSize::bytes(-50);
    let positive_result = negative_size * -1i64;
    assert_eq!(positive_result, 50);
}

#[test]
fn test_datasize_arithmetic_with_negatives() {
    let positive = DataSize::bytes(100);
    let negative = DataSize::bytes(-50);

    // Add negative to positive
    let result = positive + negative;
    assert_eq!(result.as_bytes_i64(), 50);

    // Subtract negative from positive (should add)
    let result2 = positive - negative;
    assert_eq!(result2.as_bytes_i64(), 150);
}
