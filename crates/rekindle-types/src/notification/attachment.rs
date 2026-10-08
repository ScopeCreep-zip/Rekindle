//! Veilid attachment state, mirrored into Tier 1.
//!
//! Split out of `notification.rs` at the 600-line ceiling. The seam is
//! real: this is upstream's network-attachment ladder and its string
//! parsing, while the parent module is our own transport-notification
//! vocabulary. They travel together in the UI and nowhere else.

use serde::{Deserialize, Serialize};

/// Network attachment state. Maps from Veilid's string representation.
///
/// This is a stable ABI contract consumed by CLI display code, and it
/// crosses the daemon->CLI IPC boundary as postcard, which encodes a
/// fieldless enum by its DECLARATION INDEX (not the `repr(u8)` value).
/// So variants may only ever be APPENDED -- inserting one in the middle
/// silently reinterprets every later variant on a version-skewed socket.
/// `AttachedFair` is appended for that reason even though it belongs
/// between `AttachedWeak` and `AttachedGood` by signal strength; use
/// [`AttachmentState::strength`] for ordering rather than the derived
/// `Ord`, which follows declaration order and is therefore NOT ordered
/// by "goodness".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum AttachmentState {
    Detached = 0,
    Attaching = 1,
    AttachedWeak = 2,
    AttachedGood = 3,
    AttachedStrong = 4,
    /// Veilid <=0.5.3 spelled this `FullyAttached`; 0.5.4+ emits
    /// `attached_full`. Both strings parse here.
    FullyAttached = 5,
    /// Removed upstream in veilid-core 0.5.4. Retained so the postcard
    /// declaration index of `Detaching` does not shift.
    OverAttached = 6,
    Detaching = 7,
    /// Added upstream in veilid-core 0.5.4. Appended, not inserted -- see
    /// the type-level note.
    AttachedFair = 8,
}

impl AttachmentState {
    /// Parse from Veilid's attachment state string representation.
    ///
    /// Unknown strings map to `Detached` (fail closed -- if we can't parse
    /// the state, we assume the worst).
    ///
    /// The explicit `Detached` arm and the `_` fallback have identical
    /// bodies by coincidence, not by meaning: one says veilid told us
    /// Detached, the other says we did not recognise what veilid told
    /// us. Merging them would delete the record that `Detached` is a
    /// string we actually know — which is what
    /// `every_veilid_attachment_string_is_understood` walks upstream's
    /// enum to verify, and that test caught the veilid-core 0.5.4
    /// `attached_full` rename.
    #[allow(clippy::match_same_arms, reason = "see the doc comment above")]
    pub fn from_veilid_string(s: &str) -> Self {
        match s {
            "Detached" | "detached" => Self::Detached,
            "Attaching" | "attaching" => Self::Attaching,
            "AttachedWeak" | "attached_weak" => Self::AttachedWeak,
            "AttachedFair" | "attached_fair" => Self::AttachedFair,
            "AttachedGood" | "attached_good" => Self::AttachedGood,
            "AttachedStrong" | "attached_strong" => Self::AttachedStrong,
            // `attached_full` is the veilid-core 0.5.4+ spelling.
            "FullyAttached" | "fully_attached" | "AttachedFull" | "attached_full" => {
                Self::FullyAttached
            }
            "OverAttached" | "over_attached" => Self::OverAttached,
            "Detaching" | "detaching" => Self::Detaching,
            _ => Self::Detached,
        }
    }

    /// Whether this state represents an attached (usable) network.
    pub fn is_attached(self) -> bool {
        matches!(
            self,
            Self::AttachedWeak
                | Self::AttachedFair
                | Self::AttachedGood
                | Self::AttachedStrong
                | Self::FullyAttached
                | Self::OverAttached
        )
    }

    /// Signal strength, ascending. Use this for ordering comparisons --
    /// the derived `Ord` follows declaration order, which is an IPC wire
    /// constraint rather than a semantic one.
    pub fn strength(self) -> u8 {
        match self {
            Self::Detached | Self::Detaching => 0,
            Self::Attaching => 1,
            Self::AttachedWeak => 2,
            Self::AttachedFair => 3,
            Self::AttachedGood => 4,
            Self::AttachedStrong => 5,
            Self::FullyAttached | Self::OverAttached => 6,
        }
    }

    /// Human-readable label for display.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Detached => "Detached",
            Self::Attaching => "Attaching",
            Self::AttachedWeak => "AttachedWeak",
            Self::AttachedFair => "AttachedFair",
            Self::AttachedGood => "AttachedGood",
            Self::AttachedStrong => "AttachedStrong",
            Self::FullyAttached => "FullyAttached",
            Self::OverAttached => "OverAttached",
            Self::Detaching => "Detaching",
        }
    }

    /// Parse from a u8 discriminant. Returns `Detached` for out-of-range values.
    pub fn from_u8(raw: u8) -> Self {
        match raw {
            1 => Self::Attaching,
            2 => Self::AttachedWeak,
            3 => Self::AttachedGood,
            4 => Self::AttachedStrong,
            5 => Self::FullyAttached,
            6 => Self::OverAttached,
            7 => Self::Detaching,
            8 => Self::AttachedFair,
            _ => Self::Detached,
        }
    }
}

impl std::fmt::Display for AttachmentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
