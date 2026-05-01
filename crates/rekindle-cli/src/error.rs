//! Error types and exit code contract for the CLI.
//!
//! Every error type carries enough context to produce an actionable message
//! following the pattern: what went wrong + why it matters + what to do.
//!
//! Exit codes are a stable contract consumed by CI, scripts, and monitoring.
//! See the exit code table in the milestone spec for the full mapping.

use std::fmt;

/// CLI-specific error types that map to distinct exit codes.
///
/// These wrap transport errors and add CLI-layer context (config issues,
/// initialization state, user input validation). The `Other` variant
/// catches everything else via `anyhow::Error`.
#[derive(Debug)]
pub enum CliError {
    /// Identity not created or node not started.
    NotInitialized(String),

    /// Network operation timed out.
    Timeout(String),

    /// Authentication or authorization failure.
    Auth(String),

    /// Transport layer error (network, DHT, crypto).
    Transport(rekindle_transport::TransportError),

    /// Configuration loading or validation error.
    Config(String),

    /// User input validation error.
    Validation(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInitialized(msg) => write!(f, "not initialized: {msg}"),
            Self::Timeout(msg) => write!(f, "timeout: {msg}"),
            Self::Auth(msg) => write!(f, "auth failed: {msg}"),
            Self::Transport(e) => write!(f, "transport: {e}"),
            Self::Config(msg) => write!(f, "config: {msg}"),
            Self::Validation(msg) => write!(f, "validation: {msg}"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rekindle_transport::TransportError> for CliError {
    fn from(e: rekindle_transport::TransportError) -> Self {
        Self::Transport(e)
    }
}

/// Map an error to an exit code.
///
/// Walks the anyhow error chain looking for typed errors. Falls back to 1
/// for unrecognized errors.
///
/// | Code | Meaning |
/// |------|---------|
/// | 0 | Success |
/// | 1 | General error |
/// | 2 | Timeout |
/// | 3 | Auth failure |
/// | 4 | Not initialized |
/// | 10 | Doctor: all pass |
/// | 11 | Doctor: failures |
/// | 12 | Doctor: warnings only |
pub fn exit_code(err: &anyhow::Error) -> i32 {
    // Check CLI-specific errors first
    if let Some(e) = err.downcast_ref::<CliError>() {
        return match e {
            CliError::Timeout(_) => 2,
            CliError::Auth(_) => 3,
            CliError::NotInitialized(_) => 4,
            // Unwrap transport errors to check for timeout/auth inside
            CliError::Transport(te) => match te {
                rekindle_transport::TransportError::Timeout { .. } => 2,
                rekindle_transport::TransportError::SignatureVerificationFailed { .. } => 3,
                rekindle_transport::TransportError::NotStarted
                | rekindle_transport::TransportError::NetworkNotReady => 4,
                _ => 1,
            },
            CliError::Config(_) | CliError::Validation(_) => 1,
        };
    }

    // Check transport errors directly (may not be wrapped in CliError)
    if let Some(e) = err.downcast_ref::<rekindle_transport::TransportError>() {
        return match e {
            rekindle_transport::TransportError::Timeout { .. } => 2,
            rekindle_transport::TransportError::SignatureVerificationFailed { .. } => 3,
            rekindle_transport::TransportError::NotStarted
            | rekindle_transport::TransportError::NetworkNotReady => 4,
            _ => 1,
        };
    }

    1
}

/// Produce a remediation hint for a given error, if one is available.
///
/// Hints are actionable commands the user can run to fix the problem.
/// Returns `None` for errors that don't have a clear remediation path.
pub fn remediation(err: &anyhow::Error) -> Option<&'static str> {
    if let Some(e) = err.downcast_ref::<CliError>() {
        return match e {
            CliError::NotInitialized(_) => Some("initialize with: rekindle init"),
            CliError::Timeout(_) => Some("check network: rekindle doctor network"),
            CliError::Auth(_) => Some("check identity: rekindle identity show"),
            CliError::Transport(te) => transport_remediation(te),
            CliError::Config(_) => Some("validate config: rekindle config validate"),
            CliError::Validation(_) => None,
        };
    }

    if let Some(te) = err.downcast_ref::<rekindle_transport::TransportError>() {
        return transport_remediation(te);
    }

    None
}

/// Remediation hints for transport-layer errors.
fn transport_remediation(te: &rekindle_transport::TransportError) -> Option<&'static str> {
    match te {
        rekindle_transport::TransportError::NotStarted => {
            Some("start the node: rekindle node start")
        }
        rekindle_transport::TransportError::NetworkNotReady => {
            Some("wait for network: rekindle status --watch")
        }
        rekindle_transport::TransportError::NoRoute { .. } => {
            Some("check peer routes: rekindle network routes --refresh")
        }
        rekindle_transport::TransportError::CircuitOpen { .. } => {
            Some("circuit breaker tripped — retry after cooldown or: rekindle network peers")
        }
        rekindle_transport::TransportError::NoMekForChannel { .. }
        | rekindle_transport::TransportError::MekNotCached { .. } => {
            Some("request MEK: rekindle key mek request <community> <channel>")
        }
        rekindle_transport::TransportError::RouteAllocationFailed { .. }
        | rekindle_transport::TransportError::AttachFailed { .. } => {
            Some("check node health: rekindle doctor node")
        }
        rekindle_transport::TransportError::IdentityCreationFailed { .. } => {
            Some("retry initialization: rekindle init")
        }
        rekindle_transport::TransportError::JoinRejected { .. } => {
            Some("check invite code and try again: rekindle community join <invite>")
        }
        rekindle_transport::TransportError::JoinTimeout { .. } => {
            Some("community may be offline — retry later: rekindle community join <invite>")
        }
        rekindle_transport::TransportError::FriendRequestFailed { .. } => {
            Some("check peer is online: rekindle friend list --status online")
        }
        rekindle_transport::TransportError::VoiceJoinFailed { .. } => {
            Some("check MEK: rekindle key mek list <community>")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_not_initialized() {
        let err: anyhow::Error = CliError::NotInitialized("no identity".into()).into();
        assert_eq!(exit_code(&err), 4);
    }

    #[test]
    fn exit_code_timeout() {
        let err: anyhow::Error = CliError::Timeout("rpc timed out".into()).into();
        assert_eq!(exit_code(&err), 2);
    }

    #[test]
    fn exit_code_auth() {
        let err: anyhow::Error = CliError::Auth("signature invalid".into()).into();
        assert_eq!(exit_code(&err), 3);
    }

    #[test]
    fn exit_code_transport_timeout() {
        let te = rekindle_transport::TransportError::Timeout {
            operation: "app_call".into(),
            duration_ms: 8000,
        };
        let err: anyhow::Error = CliError::Transport(te).into();
        assert_eq!(exit_code(&err), 2);
    }

    #[test]
    fn exit_code_general() {
        let err = anyhow::anyhow!("some random error");
        assert_eq!(exit_code(&err), 1);
    }

    #[test]
    fn remediation_not_initialized() {
        let err: anyhow::Error = CliError::NotInitialized("no identity".into()).into();
        assert_eq!(remediation(&err), Some("initialize with: rekindle init"));
    }

    #[test]
    fn remediation_transport_not_started() {
        let te = rekindle_transport::TransportError::NotStarted;
        let err: anyhow::Error = CliError::Transport(te).into();
        assert_eq!(remediation(&err), Some("start the node: rekindle node start"));
    }

    #[test]
    fn remediation_unknown_error_returns_none() {
        let err = anyhow::anyhow!("unknown");
        assert_eq!(remediation(&err), None);
    }

    // ── Every CliError variant produces correct exit code ───────────

    #[test]
    fn exit_code_config() {
        let err: anyhow::Error = CliError::Config("parse failed".into()).into();
        assert_eq!(exit_code(&err), 1);
    }

    #[test]
    fn exit_code_validation() {
        let err: anyhow::Error = CliError::Validation("bad input".into()).into();
        assert_eq!(exit_code(&err), 1);
    }

    // ── Transport error variants unwrap correctly ──────────────────

    #[test]
    fn exit_code_transport_not_started() {
        let te = rekindle_transport::TransportError::NotStarted;
        let err: anyhow::Error = CliError::Transport(te).into();
        assert_eq!(exit_code(&err), 4);
    }

    #[test]
    fn exit_code_transport_network_not_ready() {
        let te = rekindle_transport::TransportError::NetworkNotReady;
        let err: anyhow::Error = CliError::Transport(te).into();
        assert_eq!(exit_code(&err), 4);
    }

    #[test]
    fn exit_code_transport_signature_failed() {
        let te = rekindle_transport::TransportError::SignatureVerificationFailed {
            sender: "abcdef".into(),
        };
        let err: anyhow::Error = CliError::Transport(te).into();
        assert_eq!(exit_code(&err), 3);
    }

    #[test]
    fn exit_code_transport_send_failed() {
        let te = rekindle_transport::TransportError::SendFailed {
            target: "peer".into(),
            reason: "dead route".into(),
        };
        let err: anyhow::Error = CliError::Transport(te).into();
        assert_eq!(exit_code(&err), 1); // general error
    }

    // ── Every transport error with remediation has a hint ───────────

    #[test]
    fn remediation_circuit_open() {
        let te = rekindle_transport::TransportError::CircuitOpen {
            peer: "abc".into(),
            failures: 3,
        };
        let err: anyhow::Error = CliError::Transport(te).into();
        assert!(remediation(&err).is_some());
    }

    #[test]
    fn remediation_no_route() {
        let te = rekindle_transport::TransportError::NoRoute {
            peer: "abc".into(),
        };
        let err: anyhow::Error = CliError::Transport(te).into();
        assert!(remediation(&err).is_some());
    }

    #[test]
    fn remediation_mek_not_cached() {
        let te = rekindle_transport::TransportError::MekNotCached {
            community: "com".into(),
            channel: "ch".into(),
            generation: 7,
        };
        let err: anyhow::Error = CliError::Transport(te).into();
        assert!(remediation(&err).is_some());
    }

    #[test]
    fn remediation_identity_creation_failed() {
        let te = rekindle_transport::TransportError::IdentityCreationFailed {
            step: "profile".into(),
            reason: "dht error".into(),
        };
        let err: anyhow::Error = CliError::Transport(te).into();
        assert!(remediation(&err).is_some());
    }

    #[test]
    fn remediation_join_rejected() {
        let te = rekindle_transport::TransportError::JoinRejected {
            community: "dev".into(),
            reason: "banned".into(),
        };
        let err: anyhow::Error = CliError::Transport(te).into();
        assert!(remediation(&err).is_some());
    }

    #[test]
    fn remediation_config_error() {
        let err: anyhow::Error = CliError::Config("bad toml".into()).into();
        assert_eq!(
            remediation(&err),
            Some("validate config: rekindle config validate")
        );
    }
}
