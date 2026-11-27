use super::*;

/// The contract: one lock per data root. A second holder is refused
/// and told who holds it; once the first is dropped, the root is free.
#[test]
fn second_holder_is_refused_until_the_first_drops() {
    let root = tempfile::tempdir().unwrap();
    let first = NodeLock::acquire(root.path()).unwrap();
    match NodeLock::acquire(root.path()) {
        Err(NodeLockError::Held { pid, .. }) => {
            assert_eq!(pid, std::process::id().to_string());
        }
        other => panic!("expected Held, got {other:?}"),
    }
    drop(first);
    NodeLock::acquire(root.path()).unwrap();
}

#[test]
fn different_roots_do_not_contend() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let _a = NodeLock::acquire(a.path()).unwrap();
    let _b = NodeLock::acquire(b.path()).unwrap();
}
