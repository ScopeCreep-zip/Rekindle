//! Error types and exit code contract for the CLI.
//!
//! `CliError` covers CLI-local failures (config, validation). Daemon
//! interaction errors are `rekindle_client::ClientError` — the CLI
//! downcasts to both in `exit_code()` and `remediation()`.
//!
//! Exit codes follow Unix convention: 0=success, 1=general, 2=timeout,
//! 3=auth, 4=state.

use std::fmt;

use rekindle_client::ClientError;

/// CLI-specific error type — local failures only.
///
/// Daemon errors go through `ClientError`. This enum exists for
/// errors that originate in the CLI process itself, not from the
/// daemon connection.
#[derive(Debug)]
pub enum CliError {
    /// Configuration file parse or validation error.
    Config(String),
    /// User input validation error (name too long, invalid format, etc.).
    Validation(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(msg) => write!(f, "config: {msg}"),
            Self::Validation(msg) => write!(f, "validation: {msg}"),
        }
    }
}

impl std::error::Error for CliError {}

/// Map an error to a process exit code.
///
/// Exit codes:
/// - 0: success
/// - 1: general error (config, validation, unknown daemon error)
/// - 2: timeout (daemon not responding, network I/O timeout)
/// - 3: auth failure (daemon locked, permission denied)
/// - 4: state error (daemon not initialized, wrong lifecycle state)
pub fn exit_code(err: &anyhow::Error) -> i32 {
    if let Some(e) = err.downcast_ref::<CliError>() {
        return match e {
            CliError::Config(_) | CliError::Validation(_) => 1,
        };
    }
    if let Some(e) = err.downcast_ref::<ClientError>() {
        return match e {
            ClientError::Timeout(_) => 2,
            ClientError::Auth(_) => 3,
            ClientError::NotInitialized(_) | ClientError::ConnectionLost(_) => 4,
            ClientError::Daemon { code, .. } => match *code {
                403 => 3,
                408 => 2,
                409 | 503 => 4,
                _ => 1,
            },
        };
    }
    1
}

/// Produce a remediation hint for a given error.
///
/// Every user-facing error should have a remediation hint. The hint
/// tells the user the single most likely action to resolve the error.
pub fn remediation(err: &anyhow::Error) -> Option<&'static str> {
    if let Some(e) = err.downcast_ref::<CliError>() {
        return match e {
            CliError::Config(_) => Some("validate config: rekindle config validate"),
            CliError::Validation(_) => None,
        };
    }
    if let Some(e) = err.downcast_ref::<ClientError>() {
        return match e {
            ClientError::NotInitialized(_) => Some("initialize with: rekindle init"),
            ClientError::Timeout(_) => Some("check daemon: systemctl status rekindle-node"),
            ClientError::Auth(_) => Some("unlock daemon: rekindle unlock"),
            ClientError::ConnectionLost(_) => Some("check daemon: rekindle node start"),
            ClientError::Daemon { code, .. } => match *code {
                403 => Some("unlock the daemon: rekindle unlock"),
                409 => Some("check daemon state: rekindle status"),
                503 => Some("start the daemon: rekindle node start"),
                _ => None,
            },
        };
    }
    None
}
