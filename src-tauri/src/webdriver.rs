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
