use super::{classify, Rule, StreamEngine};

#[test]
fn the_receive_rule_applies_the_next_passes_older_and_holds_a_gap() {
    assert_eq!(classify(0, 1), Rule::Apply);
    assert_eq!(classify(4, 5), Rule::Apply);
    assert_eq!(classify(4, 4), Rule::Duplicate);
    assert_eq!(classify(4, 2), Rule::Duplicate);
    assert_eq!(classify(4, 7), Rule::Hold);
    assert_eq!(classify(u64::MAX, 1), Rule::Duplicate);
}

/// One diff per stream: a trigger during it makes it read once more, and
/// another stream runs alongside.
#[test]
fn a_trigger_during_a_diff_coalesces_into_one_more_pass() {
    let engine = StreamEngine::default();
    let a = ("dm".to_string(), "alice".to_string());
    let b = ("dm".to_string(), "bob".to_string());
    assert!(engine.claim(&a));
    assert!(engine.claim(&b));
    assert!(!engine.claim(&a));
    assert!(!engine.claim(&a));
    assert!(!engine.release(&a, false), "read again");
    assert!(engine.release(&a, false), "drained");
    assert!(engine.claim(&a));
    assert!(!engine.claim(&a));
    assert!(engine.release(&a, true), "a failed pass ends the diff");
    assert!(engine.release(&b, false));
}
