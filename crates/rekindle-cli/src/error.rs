//! Error types and exit code contract for the CLI.

use std::fmt;

#[derive(Debug)]
pub enum CliError {
    /// Daemon not running or identity not created.
    NotInitialized(String),
    /// Operation timed out.
    Timeout(String),
    /// Authentication or authorization failure.
    Auth(String),
    /// Daemon returned an error response.
    Daemon { code: u32, message: String },
    /// Configuration error.
    Config(String),
    /// Input validation error.
    Validation(String),
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
        }
    }
}

impl std::error::Error for CliError {}

/// Map an error to an exit code.
pub fn exit_code(err: &anyhow::Error) -> i32 {
    if let Some(e) = err.downcast_ref::<CliError>() {
        return match e {
            CliError::Timeout(_) => 2,
            CliError::Auth(_) => 3,
            CliError::NotInitialized(_) => 4,
            CliError::Daemon { code, .. } => match *code {
                403 => 3,
                409 | 503 => 4,
                _ => 1,
            },
            CliError::Config(_) | CliError::Validation(_) => 1,
        };
    }
    1
}

/// Produce a remediation hint for a given error.
pub fn remediation(err: &anyhow::Error) -> Option<&'static str> {
    if let Some(e) = err.downcast_ref::<CliError>() {
        return match e {
            CliError::NotInitialized(_) => Some("initialize with: rekindle init"),
            CliError::Timeout(_) => Some("check daemon: systemctl status rekindle-node"),
            CliError::Auth(_) => Some("unlock daemon: rekindle unlock"),
            CliError::Daemon { code, .. } => match *code {
                403 => Some("unlock the daemon: rekindle unlock"),
                409 => Some("check daemon state: rekindle status"),
                503 => Some("start the daemon: rekindle-node"),
                _ => None,
            },
            CliError::Config(_) => Some("validate config: rekindle config validate"),
            CliError::Validation(_) => None,
        };
    }
    None
}
