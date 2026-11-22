//! System-level events — announcements, alerts, bootstrap, sync, kicked.

use serde::{Deserialize, Serialize};

/// System-level events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SystemEvent {
    /// **This peer** observed the join rate crossing the community's
    /// policy threshold.
    ///
    /// Architecture §20.6: every peer runs the detector, so this is a
    /// local observation rather than a broadcast. The counts are carried so a client can say *how* far
    /// over the threshold it is rather than just that it happened.
    RaidDetected {
        community: String,
        joins_in_window: u32,
        max_joins_per_interval: u32,
        join_interval_seconds: u32,
    },

    /// A local AutoMod rule matched a message.
    ///
    /// Local to the observing client, like [`Self::RaidDetected`]:
    /// automod runs per-peer, so this is what *we* flagged.
    AutoModAlert {
        community: String,
        channel: String,
        message_id: String,
        rule_name: String,
    },
    /// A bootstrap request was received (operator processing a new joiner).
    /// Triggered by: gossip `ControlPayload::BootstrapRequest`.
    BootstrapRequested {
        community: String,
        joiner_pseudonym: String,
    },
    /// A bootstrap response was received (new joiner receiving initial state).
    /// Triggered by: the inviter's `ControlPayload::BootstrapResponseV2`.
    BootstrapReceived { community: String },
    /// Phase 4 — local audit chain failed integrity check. The on-disk
    /// `audit_entries` table was tampered with, truncated, or corrupted
    /// at or after the given cursor. Triggered by: `audit_verify` Tauri
    /// command or boot-time chain check. Frontend surfaces as a typed
    /// `notification-event` toast so the user knows their device's
    /// integrity guarantees were violated.
    AuditChainBroken { cursor: u64 },
    /// The skin this device shows changed (plan E7.7.2): the identity
    /// chose one, or set the device login skin. Every window, the login
    /// window included, swaps its stylesheet and `data-skin-base`.
    /// Triggered by: `SkinSelect`, `SkinSetLogin`.
    SkinChanged {
        skin: crate::dto::skin_library::SkinApplied,
    },
    /// A library skin went on trial (`Some`: the window that shows the
    /// Settings renders "Keep this skin?" with its countdown), or its
    /// trial ended (`None`: kept, reverted, replaced or the identity
    /// locked). Triggered by: `SkinSelect` of a `pkg:` id, `SkinKeep`, the
    /// Host's revert after `SKIN_TRIAL_MS`.
    SkinTrialChanged {
        trial: Option<crate::dto::skin_library::SkinTrial>,
    },
    /// The quick wipe was armed (`armed`, with its panic shortcut or none)
    /// or disarmed (plan E7.1). The GUI registers or releases the panic
    /// shortcut's OS grab (or portal binding) on it, the path its unlock
    /// takes; the TUI and CLI, which have no global grab, show the state.
    /// Triggered by: the committed `QuickWipeArm`, `QuickWipeDisarm`.
    QuickWipeChanged {
        armed: bool,
        shortcut: Option<String>,
    },
    /// New entries in the local activity feed (plan E7.15), newest first,
    /// names resolved by the Host: the Infoview's Activity section adds
    /// them without asking again. Triggered by: a friend's presence, game,
    /// name, status message or bio change, a new friendship, while
    /// `keep_activity_log` is on.
    ActivityAppended {
        entries: Vec<crate::activity::ActivityListItem>,
    },
    /// We added a favourite game server (plan E7.2): a window listing
    /// them reads them again (`FavouriteServersGet`).
    /// Triggered by: `FavouriteServerAdd`, once stored.
    FavouriteServerAdded {
        game_id: u32,
        server: String,
        label: String,
    },
    /// A window's Infoview composed again (plan E7.15, IV4; design §2.2),
    /// delivered to that window only (ADR 0013).
    /// Triggered by: the Host's Infoview worker, folding the journal events
    /// its subject shows (a friend's presence, name, bio or friendship, a
    /// feed entry, an unread or membership change), coalesced per window.
    Infoview(crate::dto::infoview::InfoviewEvent),
    /// A skin window's state signals changed (plan SK2-A4 step 3): its
    /// focus, the selection's capabilities, typing, ringing or talking,
    /// the network, a new notification. Only that window applies it.
    /// Triggered by: any of those inputs changing (the Host's
    /// `skin::signals` worker); kept as the session state
    /// `skin-signals:<label>`, so a window that opens is handed it.
    /// `window` is the window's label (lead decision 48): signals are
    /// per window, not per window kind.
    SkinSignals {
        window: String,
        set: crate::dto::skin_signals::SkinSignalSet,
    },
}
