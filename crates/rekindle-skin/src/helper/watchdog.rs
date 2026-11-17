//! The macOS footprint watchdog (plan E7.7.1 "Helper process", d4 §2.2).
//!
//! XNU enforces `RLIMIT_AS`, but an arm64 process already maps hundreds of
//! GiB, so any meaningful cap is EINVAL, and the jetsam limits are private
//! SPI. The Host therefore reads the running helper's `ri_phys_footprint`
//! (the figure Activity Monitor shows, compressed memory included) through
//! `libproc`'s safe `pidrusage` on every poll, and the caller kills it over
//! the cap. This is a soft cap with a polling window, never a hard one.

use std::process::Child;

use libproc::libproc::pid_rusage::{pidrusage, RUsageInfoV2};

use super::HelperError;

/// Passes while the running `child`'s footprint is at most `cap`.
///
/// # Errors
/// [`HelperError::MemoryExceeded`] over the cap; [`HelperError::Pipe`] when
/// the footprint of a child that is still running cannot be read, so the
/// helper never runs unwatched.
pub(super) fn check(child: &mut Child, cap: u64) -> Result<(), HelperError> {
    let pid = i32::try_from(child.id())
        .map_err(|_| HelperError::Pipe(format!("helper pid {} out of range", child.id())))?;
    match pidrusage::<RUsageInfoV2>(pid) {
        Ok(usage) if usage.ri_phys_footprint > cap => Err(HelperError::MemoryExceeded {
            footprint: usage.ri_phys_footprint,
            cap,
        }),
        Ok(_) => Ok(()),
        // It may have exited since `try_wait`: the next poll reaps it.
        Err(_) if !matches!(child.try_wait(), Ok(None)) => Ok(()),
        Err(e) => Err(HelperError::Pipe(format!(
            "cannot read the helper's footprint: {e}"
        ))),
    }
}
