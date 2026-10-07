//! Error types and exit code contract for the CLI.

use std::fmt;

use rekindle_client::ClientError;

/// A command line for this binary: `cmd!("init")` → `"<bin> init"`.
macro_rules! cmd {
    ($args:literal) => {
        concat!(env!("CARGO_BIN_NAME"), " ", $args)
    };
}
pub(crate) use cmd;

#[derive(Debug)]
pub enum CliError {
    /// The command exists but does nothing yet.
    Unimplemented(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unimplemented(what) => write!(f, "not implemented yet: {what}"),
        }
    }
}

impl std::error::Error for CliError {}

/// Map an error to an exit code: 0 success, 1 error, 2 timeout, 3 auth,
/// 4 daemon state (not running, 409, 503), 69 not implemented.
pub fn exit_code(err: &anyhow::Error) -> i32 {
    if let Some(e) = err.downcast_ref::<CliError>() {
        // sysexits(3) EX_UNAVAILABLE: "A service is unavailable".
        return match e {
            CliError::Unimplemented(_) => 69,
        };
    }
    match err.downcast_ref::<ClientError>() {
        Some(ClientError::Timeout { .. }) => 2,
        // The daemon's "not implemented" is the same promise as ours.
        Some(ClientError::Daemon { code: 501, .. }) => 69,
        Some(ClientError::Daemon { code: 403, .. }) => 3,
        Some(
            ClientError::NotRunning { .. }
            | ClientError::ConnectionLost
            | ClientError::Daemon {
                code: 409 | 503, ..
            },
        ) => 4,
        _ => 1,
    }
}

/// What to do about an error: the daemon's own remediation when it sent
/// one, otherwise this binary's command for the failure.
pub fn remediation(err: &anyhow::Error) -> Option<String> {
    let local = match err.downcast_ref::<ClientError>()? {
        ClientError::Daemon {
            remediation: Some(hint),
            ..
        } => return Some(hint.clone()),
        ClientError::NotRunning { .. } => cmd!("node start"),
        ClientError::Timeout { .. } | ClientError::ConnectionLost => cmd!("status"),
        ClientError::Daemon { code, .. } => match *code {
            403 => cmd!("unlock"),
            409 | 503 => cmd!("status"),
            _ => return None,
        },
        ClientError::ConfigLayers(_) | ClientError::Config(_) => cmd!("config validate"),
        _ => return None,
    };
    Some(format!("run: {local}"))
}

/// The error as JSON, for `--format json|jsonl`: printed on stdout with a
/// non-zero exit, so scripts parse one stream.
pub fn to_json(err: &anyhow::Error) -> serde_json::Value {
    let code = match err.downcast_ref::<ClientError>() {
        Some(ClientError::Daemon { code, .. }) => Some(*code),
        _ => None,
    };
    serde_json::json!({
        "error": {
            "message": format!("{err:#}"),
            "daemon_code": code,
            "remediation": remediation(err),
            "exit_code": exit_code(err),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_binary_is_the_cli_the_frontends_name() {
        assert_eq!(env!("CARGO_BIN_NAME"), rekindle_client::CLI_BIN);
    }

    #[test]
    fn unimplemented_exits_69() {
        let err = anyhow::anyhow!(CliError::Unimplemented("node attach".into()));
        assert_eq!(exit_code(&err), 69);
    }

    #[test]
    fn the_daemons_remediation_wins() {
        let err = anyhow::anyhow!(ClientError::Daemon {
            code: 503,
            message: "resume failed".into(),
            remediation: Some("check the network".into()),
        });
        assert_eq!(remediation(&err).as_deref(), Some("check the network"));
        assert_eq!(exit_code(&err), 4);
    }

    #[test]
    fn local_hints_name_this_binary() {
        let err = anyhow::anyhow!(ClientError::Timeout { secs: 5 });
        assert_eq!(
            remediation(&err).as_deref(),
            Some(concat!("run: ", cmd!("status")))
        );
        assert_eq!(exit_code(&err), 2);
    }
}
