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
//! [`STOP_DEADLINE`]: past it the process ends, whatever the shutdown still
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
pub(crate) const STOP_DEADLINE: Duration = Duration::from_secs(60);

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
        STOP_DEADLINE,
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
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::sync::mpsc;

    use super::{quit_on_stop, StopSource};

    /// Signals the test sends, counted as the shell reads them.
    struct Fed {
        signals: mpsc::UnboundedReceiver<&'static str>,
        read: Arc<AtomicUsize>,
    }

    impl StopSource for Fed {
        async fn next(&mut self) -> &'static str {
            match self.signals.recv().await {
                Some(signal) => {
                    self.read.fetch_add(1, Ordering::SeqCst);
                    signal
                }
                None => std::future::pending().await,
            }
        }
    }

    async fn until(what: &str, done: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !done() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
    }

    /// A stop signal quits the app; a second one during the shutdown is
    /// read and does not quit again.
    #[tokio::test]
    async fn the_first_stop_signal_quits_once() {
        let (tx, signals) = mpsc::unbounded_channel();
        let read = Arc::new(AtomicUsize::new(0));
        let quits = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&quits);
        let task = tokio::spawn(quit_on_stop(
            Fed {
                signals,
                read: Arc::clone(&read),
            },
            move || {
                counted.fetch_add(1, Ordering::SeqCst);
            },
            Duration::from_secs(3600),
            || panic!("no forced end inside the deadline"),
        ));

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(quits.load(Ordering::SeqCst), 0, "no signal, no quit");

        tx.send("SIGTERM").expect("send");
        until("the quit", || quits.load(Ordering::SeqCst) == 1).await;
        tx.send("SIGINT").expect("send");
        tx.send("SIGTERM").expect("send");
        until("the later signals", || read.load(Ordering::SeqCst) == 3).await;
        assert_eq!(
            quits.load(Ordering::SeqCst),
            1,
            "one quit for three signals"
        );
        assert!(!task.is_finished(), "the handler keeps reading");
        task.abort();
    }

    /// A stop signal ends the process once the deadline passes, even when
    /// the graceful shutdown never finishes (IC2); no signal, no end.
    #[tokio::test]
    async fn a_stop_signal_ends_the_process_after_the_deadline() {
        let (tx, signals) = mpsc::unbounded_channel();
        let forced = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&forced);
        let task = tokio::spawn(quit_on_stop(
            Fed {
                signals,
                read: Arc::new(AtomicUsize::new(0)),
            },
            || {},
            Duration::from_millis(50),
            move || {
                counted.fetch_add(1, Ordering::SeqCst);
            },
        ));

        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(forced.load(Ordering::SeqCst), 0, "no signal, no forced end");

        tx.send("SIGTERM").expect("send");
        until("the forced end", || forced.load(Ordering::SeqCst) == 1).await;
        task.abort();
    }
}
