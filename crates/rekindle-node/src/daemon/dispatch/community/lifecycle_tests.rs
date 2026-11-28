//! Plan E3.2 join step 5: a refused join answers the Host call with the
//! inviter's typed reason, never a bare 500.

use rekindle_ipc::protocol::IpcResponse;
use rekindle_types::join_refusal::JoinRefusal;

use super::join_failure_response;
use crate::host::adapters::governance::JoinFailure;

/// A used-up single-use invite is 409 `INVITE_USED_UP` with its message
/// and remediation; an untyped failure stays a 500.
#[test]
fn a_refused_join_returns_its_typed_reason() {
    let IpcResponse::Error {
        code,
        message,
        remediation,
    } = join_failure_response(JoinFailure::Refused(JoinRefusal::InviteUsedUp))
    else {
        panic!("a refused join is an error");
    };
    assert_eq!(code, 409);
    assert_eq!(message, JoinRefusal::InviteUsedUp.message());
    assert_eq!(message, "This invite has already been used.");
    assert_eq!(
        remediation.as_deref(),
        Some(JoinRefusal::InviteUsedUp.remediation())
    );
    let IpcResponse::Error { code, .. } = join_failure_response(JoinFailure::Failed("x".into()))
    else {
        panic!("a failed join is an error");
    };
    assert_eq!(code, 500);
}
