use rekindle_transport_ipc::v3::crypto::noise::{build_prologue, PrologueError};
use rekindle_transport_ipc::v3::wire::constants::PROLOGUE_PREFIX;

#[test]
fn prologue_starts_with_prefix() {
    let prologue = build_prologue(100, 1000, 200, 1000).unwrap();
    let s = std::str::from_utf8(&prologue).unwrap();
    assert!(s.starts_with(PROLOGUE_PREFIX), "got: {s}");
}

#[test]
fn prologue_sorts_pids_lower_first() {
    let prologue = build_prologue(100, 1000, 50, 1000).unwrap();
    let s = std::str::from_utf8(&prologue).unwrap();
    // lower PID (50) should appear before higher PID (100)
    let after_prefix = &s[PROLOGUE_PREFIX.len()..];
    let parts: Vec<&str> = after_prefix.split(':').collect();
    assert_eq!(parts.len(), 4, "expected 4 parts: lower_pid:lower_uid:higher_pid:higher_uid");
    assert_eq!(parts[0], "50");
    assert_eq!(parts[2], "100");
}

#[test]
fn prologue_is_deterministic() {
    let a = build_prologue(100, 1000, 200, 1000).unwrap();
    let b = build_prologue(100, 1000, 200, 1000).unwrap();
    assert_eq!(a, b);
}

#[test]
fn prologue_is_canonical_regardless_of_caller_role() {
    let dialler_first = build_prologue(100, 1000, 200, 1000).unwrap();
    let listener_first = build_prologue(200, 1000, 100, 1000).unwrap();
    assert_eq!(dialler_first, listener_first);
}

#[test]
fn prologue_rejects_pid_zero_local() {
    let err = build_prologue(0, 1000, 200, 1000).unwrap_err();
    assert!(
        matches!(err, PrologueError::PidZero { which: "local" }),
        "local PID 0 must produce PidZero{{local}}, got: {err:?}"
    );
}

#[test]
fn prologue_rejects_pid_zero_remote() {
    let err = build_prologue(100, 1000, 0, 1000).unwrap_err();
    assert!(
        matches!(err, PrologueError::PidZero { which: "remote" }),
        "remote PID 0 must produce PidZero{{remote}}, got: {err:?}"
    );
}

#[test]
fn prologue_uses_unpadded_decimal() {
    let prologue = build_prologue(7, 42, 1234, 5678).unwrap();
    let s = std::str::from_utf8(&prologue).unwrap();
    // "7" not "07" or "007"
    assert!(s.contains(":7:"), "PID 7 should be unpadded, got: {s}");
    assert!(s.contains(":42:"), "UID 42 should be unpadded, got: {s}");
}

#[test]
fn prologue_with_same_pid_and_uid() {
    let prologue = build_prologue(100, 1000, 100, 1000).unwrap();
    let s = std::str::from_utf8(&prologue).unwrap();
    // Both PIDs are 100, both UIDs are 1000 — canonical ordering still works
    assert!(s.starts_with(PROLOGUE_PREFIX));
}

#[test]
fn prologue_sorts_by_pid_not_uid() {
    // PID 50 with UID 9999, PID 100 with UID 1
    // Sort by PID, not UID
    let prologue = build_prologue(100, 1, 50, 9999).unwrap();
    let s = std::str::from_utf8(&prologue).unwrap();
    let after_prefix = &s[PROLOGUE_PREFIX.len()..];
    let parts: Vec<&str> = after_prefix.split(':').collect();
    assert_eq!(parts[0], "50", "lower PID must come first");
    assert_eq!(parts[1], "9999", "lower PID's UID follows");
}
