//! The validator in its own process (plan E7.7.1 "Helper process").
//!
//! The Host never decodes a skin in its own address space: [`validate`]
//! starts `rekindle-skin-helper` beside the running executable (the
//! `rekindle-client` spawn pattern), writes the package on its stdin and
//! reads one [`frame`] from its stdout under a wall-clock deadline. A panic,
//! a signal, a non-zero exit, the deadline or an answer that does not
//! decode is a [`HelperError`] of that process, never a partial skin.
//!
//! The decoders' own limits are the primary bound; the OS caps the helper
//! sets on itself ([`cap_this_process`]) are the second.

pub mod frame;

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use rekindle_types::dto::skin::{SkinError, SkinRule};

use crate::limits::MAX_PACKAGE_BYTES;
use crate::output::Canonical;

/// The helper's file name, without the platform suffix.
pub const HELPER_NAME: &str = "rekindle-skin-helper";
/// The wall clock a validation may take before the helper is killed: over
/// [`CPU_SECONDS`], so a busy helper meets its CPU cap first.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// The CPU seconds the helper may spend (`RLIMIT_CPU` soft; SIGXCPU).
pub const CPU_SECONDS: u64 = 20;
/// The helper's address space on Linux (`RLIMIT_AS`): the 16 MiB input,
/// its canonical copy, a 64 MiB decode allocation and the binary's own
/// mappings, with room to spare.
pub const ADDRESS_SPACE_BYTES: u64 = 1024 * 1024 * 1024;
/// How much of the helper's stderr a failure keeps.
const MAX_STDERR_BYTES: u64 = 16 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// A validation that did not produce a canonical package.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HelperError {
    /// The skin broke a rule: the same refusal [`crate::validate::run`]
    /// gives in process.
    #[error(transparent)]
    Refused(SkinError),
    /// The helper could not be found or started.
    #[error("skin helper could not start: {0}")]
    Spawn(String),
    /// Writing the package or reading the answer failed.
    #[error("skin helper pipe failed: {0}")]
    Pipe(String),
    /// The helper ran past the deadline and was killed.
    #[error("skin helper killed after {0:?}")]
    TimedOut(Duration),
    /// The helper died: a panic, a signal (its CPU cap included) or any
    /// other non-zero exit. `stderr` is the start of what it printed.
    #[error("skin helper failed ({status}): {stderr}")]
    Crashed { status: String, stderr: String },
    /// The helper exited cleanly but its answer does not decode.
    #[error("skin helper answer refused: {0}")]
    Malformed(String),
}

/// Where the helper is and how long it may run.
#[derive(Debug, Clone)]
pub struct HelperOpts {
    pub path: PathBuf,
    pub timeout: Duration,
}

impl HelperOpts {
    /// `rekindle-skin-helper` beside the running executable, as every
    /// package ships it.
    ///
    /// # Errors
    /// The executable path is unknown, or the helper is not beside it.
    pub fn beside_current_exe() -> Result<Self, HelperError> {
        let exe = std::env::current_exe()
            .map_err(|e| HelperError::Spawn(format!("cannot locate this executable: {e}")))?;
        let dir = exe.parent().ok_or_else(|| {
            HelperError::Spawn(format!("{} has no parent directory", exe.display()))
        })?;
        let path = dir.join(format!("{HELPER_NAME}{}", std::env::consts::EXE_SUFFIX));
        if !path.is_file() {
            return Err(HelperError::Spawn(format!(
                "{HELPER_NAME} not found at {}",
                path.display()
            )));
        }
        Ok(Self {
            path,
            timeout: DEFAULT_TIMEOUT,
        })
    }
}

/// Validates `bytes` in the helper at `opts.path`.
///
/// A package over the size limit is refused here with the refusal the
/// pipeline gives, and never reaches the helper.
///
/// # Errors
/// [`HelperError::Refused`] for a skin that breaks a rule; any other
/// variant for a helper that did not answer.
pub fn validate(bytes: &[u8], opts: &HelperOpts) -> Result<Canonical, HelperError> {
    let size = bytes.len() as u64;
    if size > MAX_PACKAGE_BYTES {
        return Err(HelperError::Refused(SkinError::new(
            SkinError::PACKAGE,
            SkinRule::PackageTooLarge {
                size,
                max: MAX_PACKAGE_BYTES,
            },
        )));
    }
    let mut child = Command::new(&opts.path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| HelperError::Spawn(format!("{}: {e}", opts.path.display())))?;
    let (Some(mut stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        kill(&mut child);
        return Err(HelperError::Pipe("a standard stream is missing".to_owned()));
    };
    std::thread::scope(|s| {
        let writer = s.spawn(move || stdin.write_all(bytes));
        let reader = s.spawn(move || read_bounded(stdout, frame::MAX_FRAME_BYTES));
        let errors = s.spawn(move || read_bounded(stderr, MAX_STDERR_BYTES));
        let status = wait(&mut child, opts.timeout);
        // The child is gone (exited or killed), so every pipe has closed
        // and each thread returns.
        let written = writer.join();
        let answer = reader.join();
        let errors = errors.join();
        let status = status?;
        let stderr = match errors {
            Ok(Ok(e)) => String::from_utf8_lossy(&e).into_owned(),
            _ => String::new(),
        };
        if !status.success() {
            return Err(HelperError::Crashed {
                status: status.to_string(),
                stderr,
            });
        }
        match written {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(HelperError::Pipe(format!("writing the package: {e}"))),
            Err(_) => return Err(HelperError::Pipe("the writer panicked".to_owned())),
        }
        let answer = match answer {
            Ok(Ok(a)) => a,
            Ok(Err(e)) => return Err(HelperError::Pipe(format!("reading the answer: {e}"))),
            Err(_) => return Err(HelperError::Pipe("the reader panicked".to_owned())),
        };
        match frame::decode(&answer).map_err(HelperError::Malformed)? {
            Ok(canonical) => Ok(canonical),
            Err(refusal) => Err(HelperError::Refused(refusal)),
        }
    })
}

/// Reads `from` to its end, keeping at most `max + 1` bytes so an
/// oversized answer is seen as one and still drained.
fn read_bounded(from: impl Read, max: u64) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut from = from;
    (&mut from).take(max + 1).read_to_end(&mut out)?;
    std::io::copy(&mut from, &mut std::io::sink())?;
    Ok(out)
}

fn kill(child: &mut Child) {
    // Already exited is the only failure, and either way it is reaped.
    let _ = child.kill();
    let _ = child.wait();
}

/// The child's exit status, or [`HelperError::TimedOut`] after killing it
/// at `timeout`.
fn wait(child: &mut Child, timeout: Duration) -> Result<ExitStatus, HelperError> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() >= deadline => {
                kill(child);
                return Err(HelperError::TimedOut(timeout));
            }
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
            Err(e) => {
                kill(child);
                return Err(HelperError::Pipe(format!("waiting for the helper: {e}")));
            }
        }
    }
}

/// The OS caps the helper sets on itself before it reads a byte (plan
/// E7.7.1, d4 §2): Linux `RLIMIT_AS` and `RLIMIT_CPU`; macOS `RLIMIT_CPU`
/// only (an arm64 process already maps hundreds of GiB, so a meaningful
/// `RLIMIT_AS` is refused with EINVAL). The hard CPU limit is one second
/// past the soft one: SIGXCPU, then SIGKILL.
///
/// # Errors
/// The kernel refused a limit; the helper must then not run.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub fn cap_this_process() -> std::io::Result<()> {
    use rlimit::{setrlimit, Resource};
    setrlimit(Resource::CPU, CPU_SECONDS, CPU_SECONDS + 1)?;
    #[cfg(target_os = "linux")]
    setrlimit(Resource::AS, ADDRESS_SPACE_BYTES, ADDRESS_SPACE_BYTES)?;
    Ok(())
}
