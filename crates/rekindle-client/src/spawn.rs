//! Starting `rekindled` on demand.
//!
//! A frontend that needs the daemon starts it, as gpg starts gpg-agent
//! ("automatically started on demand … no reason to start it manually",
//! GnuPG manual). The daemon's node lock settles a race between two
//! frontends starting it at once: the loser exits, and the frontend keeps
//! polling the bus until the winner answers or the deadline passes.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::{ClientError, DaemonClient};

/// How long a start may take before the bus answers.
pub const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Where `rekindled` is and how long to wait for it.
#[derive(Debug, Clone)]
pub struct SpawnOpts {
    pub daemon_path: PathBuf,
    pub ready_timeout: Duration,
}

impl SpawnOpts {
    /// `rekindled` beside the running executable, as every package ships it.
    ///
    /// # Errors
    /// The executable path is unknown, or `rekindled` is not beside it.
    pub fn beside_current_exe() -> Result<Self, ClientError> {
        let exe = std::env::current_exe()
            .map_err(|e| ClientError::Spawn(format!("cannot locate this executable: {e}")))?;
        let dir = exe.parent().ok_or_else(|| {
            ClientError::Spawn(format!("{} has no parent directory", exe.display()))
        })?;
        let daemon_path = dir.join(format!("rekindled{}", std::env::consts::EXE_SUFFIX));
        if !daemon_path.is_file() {
            return Err(ClientError::Spawn(format!(
                "rekindled not found at {} — install the rekindle-daemon package",
                daemon_path.display()
            )));
        }
        Ok(Self {
            daemon_path,
            ready_timeout: DEFAULT_READY_TIMEOUT,
        })
    }
}

/// What a detached start found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    /// A daemon was already answering.
    AlreadyRunning,
    /// This call started one.
    Spawned { pid: u32 },
}

/// Connect to the daemon, starting it first if none is running.
///
/// # Errors
/// The daemon cannot be started or does not answer within the deadline.
pub async fn connect_or_spawn(opts: &SpawnOpts) -> Result<DaemonClient, ClientError> {
    match DaemonClient::connect().await {
        Ok(client) => {
            // A daemon still starting answers 503 until its subscriber is up.
            client.shutdown().await;
            wait_until_answering(Instant::now() + opts.ready_timeout).await?;
        }
        Err(ClientError::NotRunning { .. }) => {
            start_detached(opts).await?;
        }
        Err(other) => return Err(other),
    }
    DaemonClient::connect().await
}

/// Whether a daemon is up and serving: it answers `Status`.
async fn answering() -> bool {
    let Ok(client) = DaemonClient::connect().await else {
        return false;
    };
    let answered = client
        .request_ok(rekindle_ipc::protocol::IpcRequest::Status)
        .await
        .is_ok();
    client.shutdown().await;
    answered
}

/// Poll until a daemon answers, or fail at `deadline`.
async fn wait_until_answering(deadline: Instant) -> Result<(), ClientError> {
    while !answering().await {
        if Instant::now() > deadline {
            return Err(ClientError::Spawn(
                "the daemon is running but did not start answering".into(),
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    Ok(())
}

/// Start `rekindled` detached and wait until its bus answers.
///
/// # Errors
/// The process cannot be spawned, or no daemon answers by the deadline.
pub async fn start_detached(opts: &SpawnOpts) -> Result<Started, ClientError> {
    if answering().await {
        return Ok(Started::AlreadyRunning);
    }
    let mut child = detached(&opts.daemon_path).spawn().map_err(|e| {
        ClientError::Spawn(format!("cannot start {}: {e}", opts.daemon_path.display()))
    })?;
    let pid = child.id();
    let started = Instant::now();
    let mut exited: Option<ExitStatus> = None;
    loop {
        if exited.is_none() {
            exited = child
                .try_wait()
                .map_err(|e| ClientError::Spawn(format!("rekindled (pid {pid}): {e}")))?;
        }
        if answering().await {
            // Ours exited because another frontend's start won the lock.
            return Ok(match exited {
                Some(_) => Started::AlreadyRunning,
                None => Started::Spawned { pid },
            });
        }
        if started.elapsed() > opts.ready_timeout {
            return Err(ClientError::Spawn(match exited {
                Some(status) => {
                    format!("rekindled exited during startup ({status}); its log says why")
                }
                None => format!(
                    "rekindled (pid {pid}) did not open its bus within {}s",
                    opts.ready_timeout.as_secs()
                ),
            }));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Become `rekindled` (Unix `exec`), or run it and wait (Windows).
///
/// # Errors
/// The process cannot be started, or exits unsuccessfully.
pub fn run_foreground(daemon_path: &Path) -> Result<(), ClientError> {
    #[cfg(unix)]
    {
        let error = std::os::unix::process::CommandExt::exec(&mut Command::new(daemon_path));
        Err(ClientError::Spawn(format!(
            "cannot exec {}: {error}",
            daemon_path.display()
        )))
    }
    #[cfg(not(unix))]
    {
        let status = Command::new(daemon_path).status().map_err(|e| {
            ClientError::Spawn(format!("cannot start {}: {e}", daemon_path.display()))
        })?;
        if status.success() {
            Ok(())
        } else {
            Err(ClientError::Spawn(format!(
                "rekindled exited with {status}"
            )))
        }
    }
}

/// A command that outlives this process: no inherited stdio, and its own
/// process group so the terminal's Ctrl-C does not reach it.
fn detached(daemon: &Path) -> Command {
    let mut command = Command::new(daemon);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    #[cfg(windows)]
    {
        // DETACHED_PROCESS: no console inherited from this terminal.
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        std::os::windows::process::CommandExt::creation_flags(&mut command, DETACHED_PROCESS);
    }
    command
}
