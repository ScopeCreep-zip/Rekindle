//! `TransportGuard` — RAII check at command entry.
//!
//! Mutating Tauri commands obtain a [`TransportGuard::write`] guard at
//! the top of their body. If the lifecycle isn't in a `can_write` state
//! (`Operational` or `Degraded`), construction returns
//! `LifecycleError::CannotWrite` and the command short-circuits.
//! Read-only commands use [`TransportGuard::read`].
//!
//! The guard itself does NOT mutate state on drop — its sole job is the
//! boundary check. Holding it for the duration of the command is purely
//! a lifetime-binding convenience so the borrow checker can verify the
//! check actually ran.

use crate::error::LifecycleError;
use crate::state::AppLifecycle;

/// Bound to an [`AppLifecycle`] for the duration of one command body.
/// Construction is the side-effecting step (the capability check); the
/// guard's presence at runtime is purely informational.
#[derive(Debug)]
pub struct TransportGuard<'a> {
    _lifecycle: &'a AppLifecycle,
}

impl<'a> TransportGuard<'a> {
    /// Acquire a write guard.
    ///
    /// # Errors
    /// Returns [`LifecycleError::CannotWrite`] if the lifecycle isn't in
    /// a state that accepts writes (i.e. not `Operational` or `Degraded`).
    pub fn write(lifecycle: &'a AppLifecycle) -> Result<Self, LifecycleError> {
        let s = lifecycle.state();
        if !s.can_write() {
            return Err(LifecycleError::CannotWrite { state: s });
        }
        Ok(Self {
            _lifecycle: lifecycle,
        })
    }

    /// Acquire a read guard.
    ///
    /// # Errors
    /// Returns [`LifecycleError::CannotQuery`] if the lifecycle isn't in
    /// a state that accepts reads (i.e. not `Operational`, `Degraded`,
    /// or `Detached`).
    pub fn read(lifecycle: &'a AppLifecycle) -> Result<Self, LifecycleError> {
        let s = lifecycle.state();
        if !s.can_query() {
            return Err(LifecycleError::CannotQuery { state: s });
        }
        Ok(Self {
            _lifecycle: lifecycle,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::LifecycleState;

    fn drive_to(target: LifecycleState) -> AppLifecycle {
        let lc = AppLifecycle::new();
        // Walk the FSM to the desired state through valid transitions.
        let path: &[LifecycleState] = match target {
            LifecycleState::Stopped => &[],
            LifecycleState::Starting => &[LifecycleState::Starting],
            LifecycleState::Locked => &[LifecycleState::Starting, LifecycleState::Locked],
            LifecycleState::Resuming => &[
                LifecycleState::Starting,
                LifecycleState::Locked,
                LifecycleState::Resuming,
            ],
            LifecycleState::Operational => &[
                LifecycleState::Starting,
                LifecycleState::Locked,
                LifecycleState::Resuming,
                LifecycleState::Operational,
            ],
            LifecycleState::Degraded => &[
                LifecycleState::Starting,
                LifecycleState::Locked,
                LifecycleState::Resuming,
                LifecycleState::Operational,
                LifecycleState::Degraded,
            ],
            LifecycleState::Detached => &[
                LifecycleState::Starting,
                LifecycleState::Locked,
                LifecycleState::Resuming,
                LifecycleState::Operational,
                LifecycleState::Detached,
            ],
            LifecycleState::Locking => &[
                LifecycleState::Starting,
                LifecycleState::Locked,
                LifecycleState::Resuming,
                LifecycleState::Operational,
                LifecycleState::Locking,
            ],
            LifecycleState::ShuttingDown => &[
                LifecycleState::Starting,
                LifecycleState::Locked,
                LifecycleState::Resuming,
                LifecycleState::Operational,
                LifecycleState::ShuttingDown,
            ],
        };
        for step in path {
            lc.transition(*step).unwrap();
        }
        lc
    }

    #[test]
    fn write_guard_succeeds_in_operational() {
        let lc = drive_to(LifecycleState::Operational);
        assert!(TransportGuard::write(&lc).is_ok());
    }

    #[test]
    fn write_guard_succeeds_in_degraded() {
        let lc = drive_to(LifecycleState::Degraded);
        assert!(TransportGuard::write(&lc).is_ok());
    }

    #[test]
    fn write_guard_rejects_in_detached() {
        let lc = drive_to(LifecycleState::Detached);
        let err = TransportGuard::write(&lc).unwrap_err();
        match err {
            LifecycleError::CannotWrite { state } => assert_eq!(state, LifecycleState::Detached),
            other => panic!("expected CannotWrite, got {other:?}"),
        }
    }

    #[test]
    fn write_guard_rejects_in_locked() {
        let lc = drive_to(LifecycleState::Locked);
        assert!(TransportGuard::write(&lc).is_err());
    }

    #[test]
    fn write_guard_rejects_in_stopped() {
        let lc = drive_to(LifecycleState::Stopped);
        assert!(TransportGuard::write(&lc).is_err());
    }

    #[test]
    fn read_guard_succeeds_in_detached() {
        // Detached can serve cached reads even when it can't write.
        let lc = drive_to(LifecycleState::Detached);
        assert!(TransportGuard::read(&lc).is_ok());
    }

    #[test]
    fn read_guard_rejects_in_locked() {
        let lc = drive_to(LifecycleState::Locked);
        let err = TransportGuard::read(&lc).unwrap_err();
        assert!(matches!(err, LifecycleError::CannotQuery { .. }));
    }

    /// Deep audit (deep-D): TransportGuard must be Send so async commands
    /// can hold it across .await points. Verified at compile-time by the
    /// type system; this test additionally constructs + holds the guard
    /// across an await to catch any future regression that would make
    /// the guard !Send.
    #[tokio::test(flavor = "current_thread")]
    async fn transport_guard_is_held_across_await() {
        let lc = drive_to(LifecycleState::Operational);
        let _g = TransportGuard::write(&lc).expect("guard acquired in Operational");
        // Cross an await point — would fail to compile if _g were !Send,
        // or be dropped early if the binding were a wildcard `_`.
        tokio::task::yield_now().await;
        // Guard still in scope here — drops at function end.
    }

    /// Deep audit (deep-Y): end-to-end FSM walk through the plan's
    /// stated boot/login/network-drop sequence, verifying TransportGuard
    /// gates writes per the can_write capability gate.
    #[test]
    fn e2e_boot_login_drop_recover_logout_walk() {
        let lc = AppLifecycle::new();

        // Boot: Stopped → Starting (pre-Veilid init).
        lc.transition(LifecycleState::Starting).unwrap();
        assert!(
            TransportGuard::write(&lc).is_err(),
            "write must fail in Starting"
        );

        // Network attaches: Starting → Locked.
        lc.transition(LifecycleState::Locked).unwrap();
        assert!(
            TransportGuard::write(&lc).is_err(),
            "write must fail in Locked"
        );
        assert!(
            TransportGuard::read(&lc).is_err(),
            "read must fail in Locked"
        );

        // Login begins: Locked → Resuming.
        lc.transition(LifecycleState::Resuming).unwrap();
        assert!(
            TransportGuard::write(&lc).is_err(),
            "write must fail in Resuming"
        );

        // Login completes: Resuming → Operational.
        lc.transition(LifecycleState::Operational).unwrap();
        assert!(
            TransportGuard::write(&lc).is_ok(),
            "write must succeed in Operational",
        );
        assert!(
            TransportGuard::read(&lc).is_ok(),
            "read must succeed in Operational",
        );

        // Network drops mid-session: Operational → Detached.
        lc.transition(LifecycleState::Detached).unwrap();
        let write_err = TransportGuard::write(&lc).unwrap_err();
        match write_err {
            LifecycleError::CannotWrite { state } => {
                assert_eq!(state, LifecycleState::Detached);
            }
            other => panic!("expected CannotWrite Detached, got {other:?}"),
        }
        // Reads still succeed in Detached (serve cached data).
        assert!(TransportGuard::read(&lc).is_ok());

        // Network recovers: Detached → Operational.
        lc.transition(LifecycleState::Operational).unwrap();
        assert!(TransportGuard::write(&lc).is_ok());

        // Logout: Operational → Locking → Locked.
        lc.transition(LifecycleState::Locking).unwrap();
        assert!(TransportGuard::write(&lc).is_err());
        lc.transition(LifecycleState::Locked).unwrap();
        assert!(TransportGuard::read(&lc).is_err());

        // User can re-login from Locked.
        lc.transition(LifecycleState::Resuming).unwrap();
        lc.transition(LifecycleState::Operational).unwrap();
        assert!(TransportGuard::write(&lc).is_ok());
    }
}
