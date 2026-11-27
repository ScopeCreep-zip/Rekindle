use super::js_number;
use serde_json::{json, Value};

#[test]
fn integral_numbers_are_json_integers() {
    assert_eq!(js_number("42"), json!(42));
    assert_eq!(js_number("-7"), json!(-7));
    assert_eq!(js_number("9223372036854775807"), json!(i64::MAX));
}

#[test]
fn a_number_past_i64_stays_a_float_and_never_saturates() {
    // 2^63: the upstream comparison let it through to a saturating cast.
    let v = js_number("9223372036854775808");
    assert!(v.is_f64(), "{v}");
    assert_eq!(v, json!(9_223_372_036_854_775_808.0_f64));
}

#[test]
fn fractions_are_floats_and_non_finite_values_null() {
    assert_eq!(js_number("1.5"), json!(1.5));
    assert_eq!(js_number("1e21"), json!(1e21_f64));
    assert_eq!(js_number("NaN"), Value::Null);
    assert_eq!(js_number("Infinity"), Value::Null);
    assert_eq!(js_number("-Infinity"), Value::Null);
}
