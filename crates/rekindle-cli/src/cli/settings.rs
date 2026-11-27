//! The identity's settings, each an on/off switch.

use clap::{Subcommand, ValueEnum};
use rekindle_types::presence::{LastSeenPrecision, PresencePolicyField, ShareScope};

/// On or off.
#[derive(Clone, Copy, ValueEnum)]
pub enum Switch {
    On,
    Off,
}

impl Switch {
    fn enabled(self) -> bool {
        matches!(self, Self::On)
    }

    pub(crate) fn members(self) -> ShareScope {
        if self.enabled() {
            ShareScope::Members
        } else {
            ShareScope::None
        }
    }
}

/// A single field of community presence consent.
#[derive(Subcommand)]
pub enum PolicyFieldCmd {
    /// Let the community see that we are online.
    ShareOnline { state: Switch },
    /// Let members see our focused channel.
    ShareLocation { state: Switch },
    /// Let members see our current activity.
    ShareActivity { state: Switch },
    /// Set last-seen precision.
    LastSeen { precision: SeenPrecision },
}

/// Last-seen choices accepted by the CLI.
#[derive(Clone, Copy, ValueEnum)]
pub enum SeenPrecision {
    Hidden,
    Coarse,
    Exact,
}

impl From<&PolicyFieldCmd> for PresencePolicyField {
    fn from(field: &PolicyFieldCmd) -> Self {
        match field {
            PolicyFieldCmd::ShareOnline { state } => Self::ShareOnline(state.enabled()),
            PolicyFieldCmd::ShareLocation { state } => Self::ShareLocation(state.members()),
            PolicyFieldCmd::ShareActivity { state } => Self::ShareActivity(state.members()),
            PolicyFieldCmd::LastSeen { precision } => Self::LastSeen(match precision {
                SeenPrecision::Hidden => LastSeenPrecision::Hidden,
                SeenPrecision::Coarse => LastSeenPrecision::Coarse,
                SeenPrecision::Exact => LastSeenPrecision::Exact,
            }),
        }
    }
}

/// The identity's settings.
#[derive(Subcommand)]
pub enum SettingsCmd {
    /// DM read receipts: friends see what we read only when both sides turn
    /// them on, and never while we are Invisible (off by default).
    ReadReceipts {
        /// `on` or `off`.
        state: Switch,
    },
    /// Share our game time with friends (off by default): turning it on
    /// sends our totals to every friend; what they received stays with
    /// them when it is turned off.
    GameTotals {
        /// `on` or `off`.
        state: Switch,
    },
    /// Scan for running games, for the local game-time history (on by
    /// default); off, no process is scanned.
    GameDetection {
        /// `on` or `off`.
        state: Switch,
    },
    /// Show friends the game we are playing (off by default); a hidden
    /// game is never shown.
    ShareGame {
        /// `on` or `off`.
        state: Switch,
    },
    /// Share which channel you're in. Omit the state to read the community policy.
    ShareLocation {
        /// Community name or anchor key.
        community: String,
        /// `on` or `off`.
        state: Option<Switch>,
    },
    /// Change a single community presence consent field.
    PresencePolicy {
        /// Community name or anchor key.
        community: String,
        #[command(subcommand)]
        field: PolicyFieldCmd,
    },
    /// Auto-away after this many idle minutes (0 off, at most 1440);
    /// turning it off while auto-away holds us Away restores our status.
    AutoAway {
        /// Minutes, 0 off.
        minutes: u32,
    },
    /// Keep a local log of what friends did, for the Infoview (on by
    /// default); off clears it.
    ActivityLog {
        /// `on` or `off`.
        state: Switch,
    },
}
