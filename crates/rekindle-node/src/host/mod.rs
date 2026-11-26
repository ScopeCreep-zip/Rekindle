//! The one backend host (ADR 0010, plan D3): `rekindled` runs it over its own
//! node, and the GUI runs it in-process over its adopted node
//! ([`runtime::Host`]). Every frontend reaches it through
//! [`rekindle_ipc::HostApi`] (implemented in [`api`]).

#[cfg(test)]
mod accept_forward_tests;
pub(crate) mod activity;
#[cfg(test)]
mod activity_tests;
pub mod adapters;
pub mod api;
pub mod audit;
pub(crate) mod audit_alert;
#[cfg(test)]
mod bio_tests;
pub mod bus_key;
mod coalesce;
pub mod community;
#[cfg(test)]
mod creator_round_tests;
#[cfg(test)]
mod direct_call_tests;
#[cfg(test)]
mod direct_deadline_tests;
#[cfg(test)]
mod direct_tests;
#[cfg(test)]
mod disappearing_tests;
mod expiry;
#[cfg(test)]
mod friend_admission_tests;
#[cfg(test)]
mod friend_answer_tests;
#[cfg(test)]
mod friend_nickname_tests;
mod friends;
pub(crate) mod game;
#[cfg(test)]
mod game_invite_tests;
pub(crate) mod game_stage;
#[cfg(test)]
mod game_totals_tests;
pub mod gate;
#[cfg(test)]
mod group_admission_tests;
#[cfg(test)]
mod group_content_tests;
#[cfg(test)]
mod group_dm_tests;
#[cfg(test)]
mod group_host_call_tests;
#[cfg(test)]
mod group_lobby_tests;
#[cfg(test)]
mod group_reply_tests;
pub mod hardening;
#[cfg(test)]
mod history_order_tests;
#[cfg(test)]
mod horizon_tests;
pub mod identity;
pub(crate) mod idle;
pub(crate) mod infoview;
#[cfg(test)]
mod infoview_tests;
mod intent_select;
pub(crate) mod journal_entry;
pub(crate) mod lobby;
#[cfg(test)]
mod local_search_tests;
pub mod media_lanes;
#[cfg(test)]
mod own_echo_tests;
pub(crate) mod own_prekeys;
pub(crate) mod own_presence;
pub(crate) mod own_profile;
mod own_records;
mod owned_record;
pub mod policy;
#[cfg(test)]
mod prekey_tests;
pub mod read_markers;
#[cfg(test)]
mod receive_tests;
#[cfg(test)]
mod reinitiate_tests;
#[cfg(test)]
mod rotation_tests;
mod route_budget;
mod route_down;
pub mod runtime;
mod serve;
pub(crate) mod servers;
#[cfg(test)]
mod servers_tests;
pub mod session;
pub mod shell_hooks;
pub(crate) mod skin;
pub(crate) mod transfers;
mod transport_config;
pub mod undo;
pub(crate) mod unlock;
pub(crate) mod unlocked;
#[cfg(all(feature = "webdriver-test", debug_assertions))]
mod webdriver;

#[cfg(test)]
pub(crate) mod fixture;
pub mod panic_shortcut;
pub mod signals;
#[cfg(any(test, feature = "test-fixtures"))]
pub mod test_fixtures;
mod wake;
pub mod watchdog;
pub mod wipe;

use rekindle_db::paths::DataRoot;

pub use runtime::{claim, ClaimedRoot, Host, HostConfig, HostMode, HostStores, TransportSource};
pub use shell_hooks::{EndReason, OwnRecordKeys, ShellHooks, UnlockedView};
pub use transport_config::user_transport_config;

/// `rekindled`'s command line. The host takes no options yet; parsing still
/// answers `--help` and `--version`.
#[derive(Debug, Clone, clap::Parser)]
#[command(name = "rekindled", version, about = "The Rekindle backend host")]
pub struct HostArgs {}

/// Log to `<log_dir>/rekindled.log` (daily rotation) through the identifier
/// scrubber every Rekindle log line passes. The returned guard flushes the
/// writer on drop and must live as long as the process.
///
/// # Errors
/// No state directory can be resolved, or the log directory cannot be
/// created. The daemon does not run unlogged.
pub fn init_tracing() -> anyhow::Result<tracing_appender::non_blocking::WorkerGuard> {
    let log_dir = DataRoot::resolve()?.logs;
    std::fs::create_dir_all(&log_dir)
        .map_err(|e| anyhow::anyhow!("cannot create log directory {}: {e}", log_dir.display()))?;
    let (writer, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(log_dir, "rekindled.log"));
    tracing_subscriber::fmt()
        .with_writer(rekindle_utils::log_scrub::ScrubbingMakeWriter::new(writer))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("rekindle=info,warn")),
        )
        .with_ansi(false)
        .init();
    Ok(guard)
}

/// A startup failure that a restart cannot fix: the policy does not parse,
/// or the bus keypair is incomplete, malformed or tampered with. `rekindled`
/// exits `EX_CONFIG` (78), which the unit lists in
/// `RestartPreventExitStatus=` so systemd does not loop on it.
#[derive(Debug)]
pub struct ConfigFault(pub anyhow::Error);

impl std::fmt::Display for ConfigFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.0)
    }
}

impl std::error::Error for ConfigFault {}

impl ConfigFault {
    /// sysexits(3) EX_CONFIG: "Something was found in an unconfigured or
    /// misconfigured state."
    pub const EXIT_CODE: u8 = 78;
}
