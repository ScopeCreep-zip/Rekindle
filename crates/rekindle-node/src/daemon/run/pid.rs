#![allow(unsafe_code)]
//! PID file management — creation, stale detection, cleanup.
//!
//! Prevents multiple daemon instances from running simultaneously.
//! The PID file is created atomically and removed on drop.

use std::path::{Path, PathBuf};

/// RAII guard for the PID file. Removes the file on drop.
pub struct PidFile {
    path: PathBuf,
}

impl PidFile {
    /// Acquire the PID file. Fails if another instance is already running.
    ///
    /// Stale detection: if the PID file exists, reads the PID and checks
    /// if the process is alive via `kill(pid, 0)`. If dead, removes the
    /// stale file and creates a fresh one.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Another daemon instance is running (PID file exists, process alive)
    /// - Cannot create/write the PID file (permissions, disk full)
    pub fn acquire(path: &Path) -> anyhow::Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            if let Ok(pid) = content.trim().parse::<u32>() {
                if process_alive(pid) {
                    anyhow::bail!(
                        "another rekindle daemon is already running (PID {pid}). \
                         If this is a stale PID file, remove it: rm {}",
                        path.display()
                    );
                }
                tracing::warn!(
                    stale_pid = pid,
                    path = %path.display(),
                    "stale PID file detected (process dead) — removing"
                );
            }
            std::fs::remove_file(path)?;
        }

        let pid = std::process::id();
        let content = format!("{pid}\n");

        // Atomic write: tmp + rename (prevents partial reads).
        let tmp = path.with_extension("pid.tmp");
        std::fs::write(&tmp, &content)?;
        std::fs::rename(&tmp, path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644));
        }

        Ok(Self {
            path: path.to_path_buf(),
        })
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        match std::fs::remove_file(&self.path) {
            Ok(()) => tracing::debug!(path = %self.path.display(), "PID file removed"),
            Err(e) => tracing::warn!(
                path = %self.path.display(),
                error = %e,
                "PID file removal failed — stale file may remain"
            ),
        }
    }
}

/// Check if a process is alive via kill(pid, 0).
#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    let Ok(pid_i32) = i32::try_from(pid) else { return false };
    // SAFETY: kill(pid, 0) with signal 0 checks process existence
    // without sending a signal. Returns 0 if process exists and we
    // have permission to send signals to it. Returns -1 with ESRCH
    // if the process does not exist.
    unsafe { libc::kill(pid_i32, 0) == 0 }
}

#[cfg(not(unix))]
fn process_alive(_pid: u32) -> bool {
    // Non-Unix: cannot reliably check. Assume alive to be safe.
    true
}
