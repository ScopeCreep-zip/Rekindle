use rekindle_transport_ipc::v3::stream::flow_control::{BackpressureState, Severity};

#[test]
fn no_backpressure_initially() {
    let bp = BackpressureState::new();
    assert!(!bp.is_blocked());
    assert!(bp.may_send());
}

#[test]
fn critical_backpressure_blocks() {
    let mut bp = BackpressureState::new();
    bp.assert_backpressure(Severity::Critical);
    assert!(bp.is_blocked());
    assert!(!bp.may_send());
}

#[test]
fn backpressure_clear_resumes() {
    let mut bp = BackpressureState::new();
    bp.assert_backpressure(Severity::Critical);
    assert!(bp.is_blocked());

    bp.clear();
    assert!(!bp.is_blocked());
    assert!(bp.may_send());
}

#[test]
fn advisory_backpressure_does_not_block() {
    let mut bp = BackpressureState::new();
    bp.assert_backpressure(Severity::Advisory);
    assert!(!bp.is_blocked(), "advisory must not block sends");
    assert!(bp.may_send());
}

#[test]
fn urgent_backpressure_blocks() {
    let mut bp = BackpressureState::new();
    bp.assert_backpressure(Severity::Urgent);
    assert!(bp.is_blocked());
}

#[test]
fn severity_is_queryable() {
    let mut bp = BackpressureState::new();
    assert_eq!(bp.current_severity(), None);
    bp.assert_backpressure(Severity::Critical);
    assert_eq!(bp.current_severity(), Some(Severity::Critical));
    bp.clear();
    assert_eq!(bp.current_severity(), None);
}

#[test]
fn escalation_to_higher_severity() {
    let mut bp = BackpressureState::new();
    bp.assert_backpressure(Severity::Advisory);
    assert!(!bp.is_blocked());
    bp.assert_backpressure(Severity::Critical);
    assert!(bp.is_blocked());
    assert_eq!(bp.current_severity(), Some(Severity::Critical));
}
