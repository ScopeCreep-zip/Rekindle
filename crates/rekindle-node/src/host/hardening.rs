//! Process hardening both secret-holding processes apply first thing in
//! `main` (plan E7.1, security roadmap 5.1): `rekindled` and the GUI, whose
//! in-process Host holds the same keys until F1.
//!
//! A core dump writes the process's memory (the unlocked identity key, the
//! vault keys, ratchet state) to disk, where it outlives the session. The
//! systemd unit's `LimitCORE=0` covers only systemd starts, so a daemon the
//! GUI spawns and the GUI itself set it here. Safe crate APIs only (no
//! `unsafe`, owner rule): rustix's `setrlimit` on macOS and Linux and, on
//! Linux, `PR_SET_DUMPABLE` 0, which also refuses `ptrace` attach by other
//! processes of the same user and keeps `/proc/<pid>/mem` root-only.
//! Windows has no safe runtime API; F3's installer excludes both
//! executables from Windows Error Reporting.

/// Forbid core dumps for the rest of the process's life: the soft and hard
/// `RLIMIT_CORE` both 0 (the hard limit cannot be raised again without
/// privilege), and on Linux the process marked not dumpable.
///
/// # Errors
/// The kernel refused a call. Lowering a limit is always permitted, so a
/// refusal means the process cannot be hardened and must not run.
#[cfg(unix)]
pub fn forbid_core_dumps() -> std::io::Result<()> {
    use rustix::process::{setrlimit, Resource, Rlimit};
    setrlimit(
        Resource::Core,
        Rlimit {
            current: Some(0),
            maximum: Some(0),
        },
    )?;
    #[cfg(target_os = "linux")]
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)?;
    Ok(())
}

/// Windows: nothing at run time (see the module docs).
///
/// # Errors
/// Never.
#[cfg(not(unix))]
pub fn forbid_core_dumps() -> std::io::Result<()> {
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn core_dumps_are_forbidden_soft_and_hard() {
        super::forbid_core_dumps().expect("harden");
        let limit = rustix::process::getrlimit(rustix::process::Resource::Core);
        assert_eq!(limit.current, Some(0));
        assert_eq!(limit.maximum, Some(0));
        #[cfg(target_os = "linux")]
        assert_eq!(
            rustix::process::dumpable_behavior().expect("read"),
            rustix::process::DumpableBehavior::NotDumpable
        );
    }
}
