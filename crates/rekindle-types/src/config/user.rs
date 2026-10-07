//! The user configuration file, `config.toml`: one strict schema read by
//! `rekindled` (its `[network]`), the TUI (its `[tui]`) and
//! `rekindle config validate` (all of it). Every table rejects unknown
//! keys, so a typo is an error wherever the file is read.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::TransportConfig;

/// The config format this build reads.
pub const CONFIG_VERSION: u32 = 1;

/// `config.toml`, after its layers are merged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserConfig {
    /// Format version of the file.
    #[serde(default = "default_config_version")]
    pub config_version: u32,

    /// Network and transport settings, consumed by `rekindled`.
    #[serde(default)]
    pub network: TransportConfig,

    /// Terminal UI settings, consumed by `rekindle-tui`.
    #[serde(default)]
    pub tui: TuiSection,
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            config_version: CONFIG_VERSION,
            network: TransportConfig::default(),
            tui: TuiSection::default(),
        }
    }
}

impl UserConfig {
    /// Check every section.
    ///
    /// # Errors
    /// The first violated constraint, naming its key.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.config_version != CONFIG_VERSION {
            return Err(ConfigError::new(
                "config_version",
                format!("{CONFIG_VERSION} (this build's format)"),
            ));
        }
        self.network.validate()?;
        self.tui.validate()
    }
}

/// `[tui]`: terminal UI settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TuiSection {
    /// Theme name (an opaline builtin or user theme).
    #[serde(default = "default_theme")]
    pub theme: String,

    /// Mouse support.
    #[serde(default = "default_true")]
    pub mouse: bool,

    /// Animations (skeleton loading, effects).
    #[serde(default = "default_true")]
    pub animations: bool,

    /// Keybinding overrides, action → key.
    #[serde(default)]
    pub keybindings: HashMap<String, String>,

    /// State-update rate in Hz.
    #[serde(default = "default_tick_rate")]
    pub tick_rate: f64,

    /// Render rate in Hz.
    #[serde(default = "default_frame_rate")]
    pub frame_rate: f64,

    /// Community selected on startup.
    #[serde(default)]
    pub default_community: Option<String>,
}

impl Default for TuiSection {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            mouse: true,
            animations: true,
            keybindings: HashMap::new(),
            tick_rate: default_tick_rate(),
            frame_rate: default_frame_rate(),
            default_community: None,
        }
    }
}

impl TuiSection {
    /// # Errors
    /// The first violated constraint, naming its `tui.` key.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.theme.is_empty() {
            return Err(ConfigError::new("tui.theme", "a theme name"));
        }
        if !(self.tick_rate > 0.0 && self.tick_rate <= 60.0) {
            return Err(ConfigError::new(
                "tui.tick_rate",
                "greater than 0, at most 60",
            ));
        }
        if !(self.frame_rate > 0.0 && self.frame_rate <= 120.0) {
            return Err(ConfigError::new(
                "tui.frame_rate",
                "greater than 0, at most 120",
            ));
        }
        Ok(())
    }
}

/// A config value outside what its key accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    key: String,
    expected: String,
}

impl ConfigError {
    pub fn new(key: impl Into<String>, expected: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            expected: expected.into(),
        }
    }

    /// The offending key, e.g. `network.gossip_ttl`.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: expected {}", self.key, self.expected)
    }
}

impl std::error::Error for ConfigError {}

fn default_config_version() -> u32 {
    CONFIG_VERSION
}
fn default_theme() -> String {
    "catppuccin-latte".into()
}
fn default_true() -> bool {
    true
}
fn default_tick_rate() -> f64 {
    4.0
}
fn default_frame_rate() -> f64 {
    30.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_file_is_valid() {
        assert!(UserConfig::default().validate().is_ok());
        let parsed: UserConfig = toml::from_str("").unwrap();
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn unknown_sections_and_keys_are_errors() {
        assert!(toml::from_str::<UserConfig>("[global]\nnamespace = \"x\"").is_err());
        assert!(toml::from_str::<UserConfig>("[tui]\nthme = \"x\"").is_err());
        assert!(toml::from_str::<UserConfig>("[network]\ngossip = 3").is_err());
    }

    #[test]
    fn sections_validate() {
        let mut cfg = UserConfig::default();
        cfg.tui.frame_rate = 0.0;
        assert_eq!(cfg.validate().unwrap_err().key(), "tui.frame_rate");
        let cfg = UserConfig {
            config_version: 2,
            ..UserConfig::default()
        };
        assert_eq!(cfg.validate().unwrap_err().key(), "config_version");
    }
}
