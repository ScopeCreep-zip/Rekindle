use super::*;

#[test]
fn not_happening_works() {
    assert_eq!(not_happening(), not_happening());
    assert!(Instant::now() < not_happening());
}

#[test]
fn already_happened_works() {
    assert_eq!(already_happened(), already_happened());
    assert!(Instant::now() > already_happened());
}

#[test]
fn already_happened_ne() {
    assert_ne!(not_happening(), already_happened());
}
