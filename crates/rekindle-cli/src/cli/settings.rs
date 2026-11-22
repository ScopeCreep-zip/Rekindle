//! The identity's settings, each an on/off switch.

use clap::{Subcommand, ValueEnum};

/// On or off.
#[derive(Clone, Copy, ValueEnum)]
pub enum Switch {
    On,
    Off,
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
    /// Keep a local log of what friends did, for the Infoview (on by
    /// default); off clears it.
    ActivityLog {
        /// `on` or `off`.
        state: Switch,
    },
}
