use super::{inbox_writer, matches, mint};
use rekindle_utils::OsRandom;

#[test]
fn the_inbox_writer_follows_the_token() {
    let (a, b) = (mint(&OsRandom), mint(&OsRandom));
    assert_eq!(inbox_writer(&a).to_bytes(), inbox_writer(&a).to_bytes());
    assert_ne!(inbox_writer(&a).to_bytes(), inbox_writer(&b).to_bytes());
}

#[test]
fn minted_tokens_are_valid_distinct_and_compare() {
    let (a, b) = (mint(&OsRandom), mint(&OsRandom));
    assert_ne!(a, b);
    assert!(matches(&a, a.as_str()));
    assert!(!matches(&a, b.as_str()));
    assert!(!matches(&a, ""));
}
