//! What every Rekindle frontend (CLI, TUI, and later the desktop) shares:
//! the connection to `rekindled`, starting it on demand, the user config,
//! and display formatting. No Veilid, no storage: a frontend that links
//! this crate links only the bus (ADR 0010).

#![forbid(unsafe_code)]
// Byte-index string slicing panics inside a multi-byte character (plan C2).
#![deny(clippy::string_slice)]

pub mod client;
pub mod config;
pub mod error;
pub mod fmt;
pub mod log;
pub mod spawn;
pub mod term;

pub use client::DaemonClient;

/// The CLI's binary name, for hints any frontend prints. The CLI's own
/// `env!("CARGO_BIN_NAME")` is tested equal to it.
pub const CLI_BIN: &str = "rekindle";

/// A CLI command line for a hint: `cli_cmd!("init")` → `"rekindle init"`.
#[macro_export]
macro_rules! cli_cmd {
    ($args:literal) => {
        concat!("rekindle ", $args)
    };
}
pub use error::ClientError;
