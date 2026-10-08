//! Why a frontend could not get an answer from the daemon.

use std::path::PathBuf;

/// A failure between a frontend and `rekindled`.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// No daemon socket exists.
    #[error("daemon not running (no socket at {})", socket.display())]
    NotRunning { socket: PathBuf },
    /// The bus was found but the connection failed.
    #[error("cannot connect to the daemon: {0}")]
    Connect(String),
    /// The daemon did not answer in time.
    #[error("no response from the daemon within {secs}s")]
    Timeout { secs: u64 },
    /// The connection dropped mid-request.
    #[error("daemon connection lost")]
    ConnectionLost,
    /// The daemon answered with an error.
    #[error("daemon ({code}): {message}")]
    Daemon {
        code: u32,
        message: String,
        /// What the daemon says to do about it.
        remediation: Option<String>,
    },
    /// The daemon answered something the protocol does not allow here.
    #[error("protocol violation: {0}")]
    Protocol(&'static str),
    /// `rekindled` could not be started, or did not come up.
    #[error("{0}")]
    Spawn(String),
    /// The log file cannot be set up.
    #[error("logging: {0}")]
    Log(String),
    /// The config file cannot be loaded.
    #[error(transparent)]
    ConfigLayers(#[from] rekindle_utils::config_layers::LayerError),
    /// The config file loaded but holds an invalid value.
    #[error("config: {0}")]
    Config(#[from] rekindle_types::config::user::ConfigError),
    /// Any other bus failure.
    #[error(transparent)]
    Ipc(rekindle_ipc::IpcError),
}
