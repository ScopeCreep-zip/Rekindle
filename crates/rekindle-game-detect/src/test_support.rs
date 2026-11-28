//! Test-only coordination shared by this crate's tests.

use parking_lot::{Mutex, MutexGuard};

/// Held by every test that scans the process table. Tests in one binary run
/// on parallel threads, and a scan opens `/proc/<pid>/stat` files while it
/// runs, so a test counting this process's open stat handles would otherwise
/// count another test's in-flight scan.
static PROCESS_SCAN: Mutex<()> = Mutex::new(());

/// Serialises process-table scans for the guard's lifetime.
pub(crate) fn process_scan() -> MutexGuard<'static, ()> {
    PROCESS_SCAN.lock()
}
