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
//!   driver requires on every request (its `server/auth.rs`);
//! - [`IDLE_SECONDS_ENV`], when set: the idle time the Host reads, fixed
//!   (0 for the specs), since the driver's input never resets the
//!   machine's own idle time and auto-away would fire mid-run.
//!
//! A spec may add [`DAMAGE_NEXT_POST_ENV`]`=1` to one app's launch: the
//! Host damages the next channel post it receives before opening it, so
//! the real damaged path runs, which no real peer can provoke
//! (`rekindle_node::host::Host::arm_damage_next_post`).

use std::sync::{Arc, OnceLock};

use rekindle_db::paths::DataRoot;
use rekindle_utils::IdleSource;
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

/// The variable fixing the idle time the Host reads, in seconds.
pub const IDLE_SECONDS_ENV: &str = "REKINDLE_WDIO_IDLE_SECONDS";

/// An idle time the walk fixed.
#[derive(Debug, Clone, Copy)]
struct FixedIdle(u64);

impl IdleSource for FixedIdle {
    fn idle_seconds(&self) -> Option<u64> {
        Some(self.0)
    }
}

/// The Host's idle port, read once at startup: [`IDLE_SECONDS_ENV`]'s
/// fixed time when it is set, the operating system's otherwise.
///
/// # Errors
/// The variable is set but is not a whole number of seconds: the app
/// refuses to start.
pub fn idle_source() -> Result<Arc<dyn IdleSource>, String> {
    match std::env::var(IDLE_SECONDS_ENV) {
        Err(std::env::VarError::NotPresent) => Ok(Arc::new(rekindle_idle::OsIdle)),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!(
            "{IDLE_SECONDS_ENV} must be a whole number of seconds"
        )),
        Ok(value) => match value.parse::<u64>() {
            Ok(secs) => Ok(Arc::new(FixedIdle(secs))),
            Err(e) => Err(format!(
                "{IDLE_SECONDS_ENV}={value:?} must be a whole number of seconds: {e}"
            )),
        },
    }
}

/// The variable arming the Host's damaged-post one-shot.
pub const DAMAGE_NEXT_POST_ENV: &str = "REKINDLE_WDIO_DAMAGE_NEXT_POST";

/// [`DAMAGE_NEXT_POST_ENV`] as read at startup.
static DAMAGE_NEXT_POST: OnceLock<bool> = OnceLock::new();

/// Parse [`DAMAGE_NEXT_POST_ENV`]'s value: `1` arms it; anything else is
/// refused.
fn parse_damage_next_post(value: &str) -> Result<bool, String> {
    match value {
        "1" => Ok(true),
        other => Err(format!(
            "{DAMAGE_NEXT_POST_ENV}: \"{other}\" is not 1 (unset it to leave posts alone)"
        )),
    }
}

/// Read [`DAMAGE_NEXT_POST_ENV`] once, at startup (off when unset).
///
/// # Errors
/// A value other than `1`, or a second read: the app refuses to start.
pub fn load_damage_next_post() -> Result<(), String> {
    let armed = match std::env::var(DAMAGE_NEXT_POST_ENV) {
        Ok(value) => parse_damage_next_post(&value)?,
        Err(std::env::VarError::NotPresent) => false,
        Err(e) => return Err(format!("{DAMAGE_NEXT_POST_ENV}: {e}")),
    };
    DAMAGE_NEXT_POST
        .set(armed)
        .map_err(|_| format!("{DAMAGE_NEXT_POST_ENV} was read twice"))
}

/// Whether the Host's damaged-post one-shot is armed for this launch.
pub fn damage_next_post() -> bool {
    DAMAGE_NEXT_POST.get().copied().unwrap_or(false)
}

#[cfg(test)]
#[path = "webdriver_tests.rs"]
mod tests;
