//! The WebDriver walk's hooks (plan E7.13 "Driver"). Compiled only with the
//! `webdriver-test` feature in a debug build (`lib.rs`), never in a release
//! (`cargo xtask check-webdriver-release`).
//!
//! The walk's runner (`e2e/walk/`) launches this build with three variables:
//! - [`DATA_ROOT_ENV`]: an existing absolute directory the app keeps all its
//!   files under ([`rekindle_db::paths::DataRoot::under`]), so the walk never
//!   opens the user's data root;
//! - `TAURI_WEBDRIVER_PORT` ([`tauri_plugin_wdio_webdriver::PORT_ENV_VAR`]):
//!   the loopback port the driver binds, chosen per run;
//! - `REKINDLE_WDIO_TOKEN`: the run's bearer token, which the vendored
//!   driver requires on every request (its `server/auth.rs`).
//!
//! A spec may add [`ANSWER_DELAY_ENV`] to one app's launch, to hold a
//! command's answer after the command's work is done (see [`delay_answer`]).

use std::sync::OnceLock;
use std::time::Duration;

use rekindle_db::paths::DataRoot;
use tauri::plugin::TauriPlugin;
use tauri::Runtime;

/// The variable naming the walk's data root.
pub const DATA_ROOT_ENV: &str = "REKINDLE_WDIO_DATA_ROOT";

/// The walk's data root.
///
/// # Errors
/// [`DATA_ROOT_ENV`] is unset, relative, or not an existing directory: the
/// app refuses to start rather than open any other root.
pub fn data_root() -> Result<DataRoot, String> {
    let base = std::env::var_os(DATA_ROOT_ENV)
        .map(std::path::PathBuf::from)
        .ok_or_else(|| format!("{DATA_ROOT_ENV} is unset"))?;
    if !base.is_absolute() || !base.is_dir() {
        return Err(format!(
            "{DATA_ROOT_ENV} must name an existing absolute directory"
        ));
    }
    Ok(DataRoot::under(&base))
}

/// The vendored WebDriver plugin on the run's port.
///
/// # Panics
/// `TAURI_WEBDRIVER_PORT` is unset or not a port: this build has no default
/// port, so a launch without the runner's variables stops here.
pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    let port = std::env::var(tauri_plugin_wdio_webdriver::PORT_ENV_VAR)
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .filter(|p| *p != 0)
        .expect("TAURI_WEBDRIVER_PORT must name the walk's loopback port");
    tauri_plugin_wdio_webdriver::init_with_port(port)
}

/// The variable holding a command's answer back (`command=ms`, comma
/// separated): `send_message=8000` answers a send 8 s after it commits.
///
/// It provokes, every run, the two orders a real network gives only
/// sometimes (plan E6.11 and peer finding F5): a send's commit-time answer
/// landing after the outbox reported the message sent, and a friends
/// snapshot landing after a friend's live status changed. The work is done
/// first and only the answer waits, so what lands is stale exactly as a
/// slow answer is. Read once at startup; a run's app exits with its spec,
/// so the delay never outlives the spec. Release builds hold none of it
/// (`cargo xtask check-webdriver-release`).
pub const ANSWER_DELAY_ENV: &str = "REKINDLE_WDIO_ANSWER_DELAY";

/// The commands whose answer may be held.
const DELAYABLE: [&str; 2] = ["send_message", "get_friends"];

/// The longest hold, so a mistyped value cannot stall a run.
const MAX_DELAY_MS: u64 = 30_000;

static ANSWER_DELAYS: OnceLock<Vec<(&'static str, Duration)>> = OnceLock::new();

/// Parse [`ANSWER_DELAY_ENV`]'s value.
///
/// # Errors
/// An entry that is not `command=ms`, a command not in [`DELAYABLE`], a
/// repeated command, or `ms` above [`MAX_DELAY_MS`].
fn parse_answer_delays(spec: &str) -> Result<Vec<(&'static str, Duration)>, String> {
    let mut out: Vec<(&'static str, Duration)> = Vec::new();
    for entry in spec.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let (name, ms) = entry
            .split_once('=')
            .ok_or_else(|| format!("{ANSWER_DELAY_ENV}: \"{entry}\" is not command=ms"))?;
        let command = DELAYABLE
            .iter()
            .copied()
            .find(|c| *c == name.trim())
            .ok_or_else(|| format!("{ANSWER_DELAY_ENV}: {name} cannot be delayed (only {DELAYABLE:?})"))?;
        let ms: u64 = ms
            .trim()
            .parse()
            .map_err(|_| format!("{ANSWER_DELAY_ENV}: \"{ms}\" is not a number of ms"))?;
        if ms > MAX_DELAY_MS {
            return Err(format!("{ANSWER_DELAY_ENV}: {ms} ms is above {MAX_DELAY_MS}"));
        }
        if out.iter().any(|(c, _)| *c == command) {
            return Err(format!("{ANSWER_DELAY_ENV}: {command} is named twice"));
        }
        out.push((command, Duration::from_millis(ms)));
    }
    Ok(out)
}

/// Read [`ANSWER_DELAY_ENV`] once, at startup (none when unset).
///
/// # Errors
/// The value does not parse: the app refuses to start rather than run a
/// spec without the order it asked for.
pub fn load_answer_delays() -> Result<(), String> {
    let delays = match std::env::var(ANSWER_DELAY_ENV) {
        Ok(spec) => parse_answer_delays(&spec)?,
        Err(std::env::VarError::NotPresent) => Vec::new(),
        Err(e) => return Err(format!("{ANSWER_DELAY_ENV}: {e}")),
    };
    ANSWER_DELAYS
        .set(delays)
        .map_err(|_| format!("{ANSWER_DELAY_ENV} was read twice"))
}

/// Hold `command`'s answer for its configured delay, if any. Called after
/// the command's work, before it returns.
pub async fn delay_answer(command: &'static str) {
    let held = ANSWER_DELAYS
        .get()
        .and_then(|d| d.iter().find(|(c, _)| *c == command))
        .map(|(_, d)| *d);
    if let Some(d) = held {
        tracing::debug!(command, ms = d.as_millis(), "holding the answer ({ANSWER_DELAY_ENV})");
        tokio::time::sleep(d).await;
    }
}

#[cfg(test)]
mod tests {
    use super::parse_answer_delays;
    use std::time::Duration;

    #[test]
    fn delays_parse_and_refuse_what_they_must() {
        assert_eq!(parse_answer_delays("").unwrap(), vec![]);
        assert_eq!(
            parse_answer_delays("send_message=8000, get_friends=0").unwrap(),
            vec![("send_message", Duration::from_millis(8000)), ("get_friends", Duration::ZERO)]
        );
        assert!(parse_answer_delays("send_message").is_err());
        assert!(parse_answer_delays("group_dm_send=10").is_err());
        assert!(parse_answer_delays("send_message=x").is_err());
        assert!(parse_answer_delays("send_message=30001").is_err());
        assert!(parse_answer_delays("send_message=1,send_message=2").is_err());
    }
}
