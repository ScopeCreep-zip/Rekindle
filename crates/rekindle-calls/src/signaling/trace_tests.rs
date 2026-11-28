use super::outcome;
use crate::error::CallError;

#[test]
fn outcome_names_ok_refusal_and_error() {
    assert_eq!(outcome::<()>(&Ok(())), "ok");
    assert_eq!(
        outcome::<()>(&Err(CallError::GroupCallRefused("calls are off".into()))),
        "refused: calls are off"
    );
    assert_eq!(
        outcome::<()>(&Err(CallError::CallNotFound("c1".into()))),
        "error: call not found: c1"
    );
}
