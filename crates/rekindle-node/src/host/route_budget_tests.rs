use super::capacity_from;
use rekindle_route::budget::BUDGET_CAPACITY;

/// Unset seeds nothing: the stored budget is attached as before.
#[test]
fn route_budget_seed_unset_is_the_stored_budget() {
    assert_eq!(capacity_from(None), Ok(None));
}

/// `0` seeds an empty bucket; the default capacity is the largest seed.
#[test]
fn route_budget_seed_names_its_tokens() {
    assert_eq!(capacity_from(Some("0")), Ok(Some(0)));
    assert_eq!(capacity_from(Some(" 3 ")), Ok(Some(3)));
    let full = BUDGET_CAPACITY.to_string();
    assert_eq!(capacity_from(Some(&full)), Ok(Some(BUDGET_CAPACITY)));
}

/// A malformed or widening value is refused, never read as unset.
#[test]
fn route_budget_seed_refuses_a_malformed_value() {
    assert!(capacity_from(Some("")).is_err());
    assert!(capacity_from(Some("-1")).is_err());
    assert!(capacity_from(Some("two")).is_err());
    let wider = (BUDGET_CAPACITY + 1).to_string();
    assert!(capacity_from(Some(&wider)).is_err());
}
