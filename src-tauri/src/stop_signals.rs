//! The process signals that quit the desktop app gracefully (plan E7.1).
//!
//! A desktop session ends by SIGTERM to each app's scope: systemd's
//! default `KillSignal=SIGTERM`, then SIGKILL after `TimeoutStopSec=`
//! (90 s by default, `systemd.kill(5)`, `systemd.service(5)`). Without a
//! handler, SIGTERM's default disposition kills the process at once: no
//! `RunEvent::Exit`, no Host stop, no Offline status, no Veilid flush. The
//! shell takes the daemon's signal set (`rekindle_node::host::signals`,
//! one implementation) and turns the first signal into `AppHandle::exit(0)`,
//! the same quit the tray's Quit makes: Tauri raises
//! `RunEvent::ExitRequested { code: Some(0) }`, which the run loop does not
//! prevent (only `code: None`, the last window closing, keeps the tray
//! alive), then `RunEvent::Exit`, which runs `shutdown::handle_exit` once.
//!
//! That shutdown awaits the Host's teardown and Veilid's flush with no cap
//! of its own (`shutdown::handle_exit`, plan C7.16), so a signal also arms
//! [`PROCESS_STOP_DEADLINE`]: past it the process ends, whatever the shutdown still
//! awaits (IC2). The deadline runs on its own OS thread, never on the
//! runtime: a runtime whose workers are busy (IC1) or a shutdown blocked on
//! one cannot hold it off.

use std::future::Future;
use std::time::Duration;

use rekindle_node::host::signals::StopSignals;

/// A source of stop signals: the process's own ([`StopSignals`]), or the
/// tests' fed source.
pub(crate) trait StopSource: Send + 'static {
    /// The name of the next stop signal received.
    fn next(&mut self) -> impl Future<Output = &'static str> + Send;
}

impl StopSource for StopSignals {
    fn next(&mut self) -> impl Future<Output = &'static str> + Send {
        StopSignals::next(self)
    }
}

/// How long a stop signal's graceful shutdown may run before the process
/// ends anyway. Past the session's own stop (`SESSION_STOP_DEADLINE`, 12 s)
/// and Veilid's wait for its in-flight calls (each up to
/// `LONGEST_VEILID_CALL`, 10 s, with no cancellation, veilid #516), and
/// inside systemd's default `TimeoutStopSec=` of 90 s (`systemd.service(5)`),
/// so the end is ours and logged, not the session manager's SIGKILL.
pub(crate) const PROCESS_STOP_DEADLINE: Duration = Duration::from_secs(60);

/// Install the stop-signal handlers and quit the app on the first signal.
/// Called once from setup, before the Host starts, so a locked app quits
/// cleanly too.
///
/// # Errors
/// A handler cannot be installed: startup stops rather than leave the app
/// stoppable only by an unclean kill (as `rekindled` does).
pub(crate) fn install(app: &tauri::AppHandle) -> Result<(), String> {
    let signals = {
        // tokio's signal driver is reached through the current runtime.
        let _runtime = tauri::async_runtime::handle().inner().enter();
        StopSignals::register().map_err(|e| format!("{e:#}"))?
    };
    let app = app.clone();
    tauri::async_runtime::spawn(quit_on_stop(
        signals,
        move || app.exit(0),
        PROCESS_STOP_DEADLINE,
        || std::process::exit(1),
    ));
    Ok(())
}

/// Wait for the first stop signal and `quit` once, and `force` the end if
/// the process still runs `deadline` later. Later signals (a second
/// Ctrl-C, systemd repeating SIGTERM) arrive while the graceful shutdown
/// runs: they are logged and read, never a second quit, and the installed
/// handler keeps them from killing the process mid-shutdown.
pub(crate) async fn quit_on_stop<S, Q, F>(mut source: S, quit: Q, deadline: Duration, force: F)
where
    S: StopSource,
    Q: FnOnce() + Send + 'static,
    F: FnOnce() + Send + 'static,
{
    let signal = source.next().await;
    tracing::info!(signal, "shutdown signal received");
    arm_deadline(deadline, force);
    quit();
    loop {
        let signal = source.next().await;
        tracing::info!(signal, "shutdown signal received while quitting");
    }
}

/// End the process with `force` once `deadline` has passed, on an OS
/// thread of its own.
fn arm_deadline<F>(deadline: Duration, force: F)
where
    F: FnOnce() + Send + 'static,
{
    let armed = std::thread::Builder::new()
        .name("rekindle-stop-deadline".into())
        .spawn(move || {
            std::thread::sleep(deadline);
            tracing::error!(
                deadline_secs = deadline.as_secs(),
                "graceful shutdown passed its deadline; the process ends now"
            );
            force();
        });
    if let Err(e) = armed {
        tracing::error!(error = %e, "stop deadline not armed: the shutdown is unbounded");
    }
}

#[cfg(test)]
#[path = "stop_signals_tests.rs"]
mod tests;
