//! `rekindle-skin-helper`: the skin validator in its own process (plan
//! E7.7.1 "Helper process").
//!
//! It caps itself (`rekindle_skin::helper::cap_this_process`), reads one
//! package from stdin (at most the package limit plus one byte), runs the
//! whole pipeline and writes one frame on stdout: the canonical package or
//! the typed refusal. It exits 0 after writing a frame; any other exit (a
//! panic is 101, a cap is a signal) is a failure of this process, which the
//! Host reports as such.

#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::process::ExitCode;

use rekindle_skin::helper::frame;
use rekindle_skin::limits::MAX_PACKAGE_BYTES;
use rekindle_skin::validate;

/// Exit status when a cap could not be set or a stream failed.
const FAILED: u8 = 2;

/// Says why on stderr (the Host keeps the start of it) and exits failed.
fn fail(why: &str) -> ExitCode {
    // Nothing is left to do if stderr itself is gone.
    let _ = writeln!(std::io::stderr().lock(), "rekindle-skin-helper: {why}");
    ExitCode::from(FAILED)
}

fn main() -> ExitCode {
    // Any panic report goes through the identifier scrubber (rule B19).
    rekindle_utils::log_scrub::install_panic_hook();
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Err(e) = rekindle_skin::helper::cap_this_process() {
        return fail(&format!("cannot set resource limits: {e}"));
    }
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin()
        .lock()
        .take(MAX_PACKAGE_BYTES + 1)
        .read_to_end(&mut input)
    {
        return fail(&format!("reading the package: {e}"));
    }
    let answer = frame::encode(&validate::run(&input));
    let mut out = std::io::stdout().lock();
    if let Err(e) = out.write_all(&answer).and_then(|()| out.flush()) {
        return fail(&format!("writing the answer: {e}"));
    }
    ExitCode::SUCCESS
}
