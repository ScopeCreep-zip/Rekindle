//! Error types and exit code contract for the CLI.
//!
//! Every daemon error code maps to a CLI exit code. Every error has a
//! remediation hint that tells the user what to do — never just what
//! went wrong. Exit codes follow Unix convention: 0=success, 1=general,
//! 2=timeout, 3=auth, 4=state.

use std::fmt;

/// CLI-specific error type. Wraps daemon errors with user-facing context.
///
/// The error message is the "what happened." The remediation is the
/// "what to do about it." Both are required for user-facing errors.
#[derive(Debug)]
pub enum CliError {
    /// Daemon not running or identity not created.
    NotInitialized(String),
    /// Operation timed out waiting for daemon response.
    Timeout(String),
    /// Authentication or authorization failure (daemon locked, wrong key).
    Auth(String),
    /// Daemon returned an error response with a typed code.
    Daemon { code: u32, message: String },
    /// Configuration file parse or validation error.
    Config(String),
    /// User input validation error (name too long, invalid format, etc.).
    Validation(String),
    /// IPC connection lost mid-operation.
    ConnectionLost(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInitialized(msg) => write!(f, "not initialized: {msg}"),
            Self::Timeout(msg) => write!(f, "timeout: {msg}"),
            Self::Auth(msg) => write!(f, "auth failed: {msg}"),
            Self::Daemon { code, message } => write!(f, "daemon ({code}): {message}"),
            Self::Config(msg) => write!(f, "config: {msg}"),
            Self::Validation(msg) => write!(f, "validation: {msg}"),
            Self::ConnectionLost(msg) => write!(f, "connection lost: {msg}"),
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
            CliError::Timeout(_) => 2,
            CliError::Auth(_) => 3,
            CliError::NotInitialized(_) | CliError::ConnectionLost(_) => 4,
            CliError::Daemon { code, .. } => match *code {
                403 => 3,
                408 => 2,
                409 | 503 => 4,
                _ => 1,
            },
            CliError::Config(_) | CliError::Validation(_) => 1,
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
            CliError::NotInitialized(_) => Some("initialize with: rekindle init"),
            CliError::Timeout(_) => Some("check daemon: systemctl status rekindle-node"),
            CliError::Auth(_) => Some("unlock daemon: rekindle unlock"),
            CliError::ConnectionLost(_) => Some("check daemon: rekindle node start"),
            CliError::Daemon { code, .. } => match *code {
                403 => Some("unlock the daemon: rekindle unlock"),
                409 => Some("check daemon state: rekindle status"),
                503 => Some("start the daemon: rekindle node start"),
                _ => None,
            },
            CliError::Config(_) => Some("validate config: rekindle config validate"),
            CliError::Validation(_) => None,
        };
    }
    None
}

/// Convert a daemon IpcResponse::Error into a CliError with the appropriate variant.
///
/// Maps error codes to specific CliError variants for correct exit code
/// and remediation hint generation. Called by `DaemonClient::request_ok()`.
pub fn from_daemon_error(code: u32, message: String) -> CliError {
    match code {
        403 => CliError::Auth(message),
        408 => CliError::Timeout(message),
        503 => CliError::NotInitialized(message),
        _ => CliError::Daemon { code, message },
    }
}
