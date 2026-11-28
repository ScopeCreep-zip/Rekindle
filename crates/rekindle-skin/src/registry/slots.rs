//! The closed icon-slot and sound-cue registries (plan E7.7.1 "Icons" and
//! "Package": `icons` and `sounds` are keyed by registry ids).
//!
//! An icon slot is a semantic id the app renders an icon by. Today those
//! are the status icons, one per status the StatusDot draws
//! (`SessionStatus` online, away, busy, offline and invisible, plus the
//! in-game display state; `src/components/status/StatusDot.tsx`). The skin
//! supplies the fill; the shape that masks it stays app-owned (WCAG 1.4.1).
//! Game icons are app data (E7.2), not skin.
//!
//! The sound cues are [`SoundCue`] from `rekindle-types`, the one list E5.2's
//! `notify_policy` decides and E7.7.4 plays, re-exported here so both
//! manifest maps are checked against one registry.

pub use rekindle_types::dto::skin::SoundCue;

/// A status icon slot of the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IconSlot {
    Online,
    Away,
    Busy,
    InGame,
    Offline,
    Invisible,
}

impl IconSlot {
    /// Every slot, in StatusDot order.
    pub const ALL: [Self; 6] = [
        Self::Online,
        Self::Away,
        Self::Busy,
        Self::InGame,
        Self::Offline,
        Self::Invisible,
    ];

    /// The slot's semantic id: the key of a manifest's `icons` and the stem
    /// of its `icons/<id>.png` entry.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Online => "status.online",
            Self::Away => "status.away",
            Self::Busy => "status.busy",
            Self::InGame => "status.ingame",
            Self::Offline => "status.offline",
            Self::Invisible => "status.invisible",
        }
    }

    /// The slot whose id is `id`, if any.
    #[must_use]
    pub fn of(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }
}
