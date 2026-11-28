//! Every Tauri command (`src-tauri/src/invoke.rs`) and every daemon
//! request (`IpcRequest::name`) with its intent kind, or `None` for a pure
//! read or plumbing call (plan U1.1). `cargo xtask check-intent-table`
//! (U1.2) fails when either side has a name the other lacks.

use super::IntentKind;

mod daemon;
mod tauri_account;
mod tauri_community;
mod tauri_core;
mod tauri_live;

/// The table, one part per domain (each part's rows are `(command or
/// request name, kind)`).
pub const COMMAND_TABLE: &[&[(&str, Option<IntentKind>)]] = &[
    tauri_core::TAURI_CORE_ROWS,
    tauri_community::TAURI_COMMUNITY_ROWS,
    tauri_live::TAURI_LIVE_ROWS,
    tauri_account::TAURI_ACCOUNT_ROWS,
    daemon::DAEMON_ROWS,
];

/// Every row of [`COMMAND_TABLE`], in order.
pub fn command_rows() -> impl Iterator<Item = &'static (&'static str, Option<IntentKind>)> {
    COMMAND_TABLE.iter().flat_map(|part| part.iter())
}

/// How the table classifies a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classified {
    Intent(IntentKind),
    /// A pure read or plumbing call.
    NotAnIntent,
}

/// The classification of a command or request, or `None` for a name the
/// table does not know.
#[must_use]
pub fn kind_of(name: &str) -> Option<Classified> {
    command_rows()
        .find(|(n, _)| *n == name)
        .map(|(_, kind)| kind.map_or(Classified::NotAnIntent, Classified::Intent))
}
