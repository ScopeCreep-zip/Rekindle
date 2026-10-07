//! The user config as a frontend reads it: every layer merged
//! (`rekindle_utils::config_layers`), then validated against the one
//! schema the daemon also reads (`rekindle_types::config::user`).

use std::path::Path;

pub use rekindle_types::config::user::{TuiSection, UserConfig};
pub use rekindle_utils::config_layers::{layers, Layer};

use crate::ClientError;

/// Load and validate the merged config; `explicit` is `--config`.
///
/// # Errors
/// A layer cannot be read or parsed, or the result is invalid.
pub fn load(explicit: Option<&Path>) -> Result<UserConfig, ClientError> {
    let config: UserConfig = rekindle_utils::config_layers::load(explicit)?;
    config.validate()?;
    Ok(config)
}
