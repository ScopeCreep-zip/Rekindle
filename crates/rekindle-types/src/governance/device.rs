//! Device-key op bodies (ADR 0022, plan E3.2 S7d-keys.1): a pseudonym's
//! endorsement of one of its per-community device keys, its retirement of
//! one, and a co-signed re-admission after a fork. The signed layouts
//! (rule 10) and the endorsement set's hash live in
//! `rekindle_governance::device`; verification against the op set is
//! S7d-keys.2.

use serde::{Deserialize, Serialize};

use crate::id::PseudonymKey;

/// At most this many element ids travel with one endorsement: the deps cap
/// (`member_record::MAX_FRONTIER_REFS`, 64), so an endorsement never
/// outgrows the op it rides in. Six live keys (rule 6) plus every retired
/// one stay far below it.
pub const MAX_DEVICE_SET_ELEMENTS: usize = 64;

/// What an element of a member's endorsement set is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceSetElementKind {
    Endorsement,
    Retirement,
}

/// One element id `(kind, deviceKey, incarnation)` of a member's set, the
/// unit `extendsSet` commits to (rule 10). Ordered by kind, then key
/// bytes, then incarnation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DeviceSetElement {
    pub kind: DeviceSetElementKind,
    pub device_key: [u8; 32],
    pub incarnation: u64,
}

impl DeviceSetElement {
    /// The element id `(kind, device_key, incarnation)`.
    #[must_use]
    pub const fn new(kind: DeviceSetElementKind, device_key: [u8; 32], incarnation: u64) -> Self {
        Self {
            kind,
            device_key,
            incarnation,
        }
    }
}

/// `DeviceKey{…}`: the pseudonym `p` endorses `device_key` for this
/// community and member record (rules 1, 2, 10). Also carried as
/// `MemberJoined`'s first device and the `GenesisCert` creator's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceEndorsement {
    pub device_key: [u8; 32],
    /// Strictly above every incarnation in `extends_elements` (Keybase's
    /// strictly greater `EldestSeqno`).
    pub incarnation: u64,
    /// The hash of `extends_elements` (`rekindle_governance::device`).
    pub extends_set: [u8; 32],
    /// The element ids of the set the signer held, strictly ascending.
    pub extends_elements: Vec<DeviceSetElement>,
    /// DHT key of the device's own member record `M(p,d)` (rule 3).
    pub device_record: String,
    /// `K_M(p,d)`'s public key text.
    pub device_record_owner: String,
    /// The fork evidence this endorsement answers, or all zero.
    pub fork_evidence: [u8; 32],
    /// `p`'s Ed25519 signature; 64 bytes.
    pub endorsement_sig: Vec<u8>,
}

impl DeviceEndorsement {
    /// Whether the body is well formed on its own: a 64-byte signature,
    /// a device record and its owner named, at most
    /// [`MAX_DEVICE_SET_ELEMENTS`] element ids, strictly ascending (one
    /// canonical list per set), and the incarnation strictly above every
    /// one of them (ADR 0022 rule 2). That `extends_set` hashes the list
    /// is checked where the hash lives (`rekindle_governance::device`).
    #[must_use]
    pub fn well_formed(&self) -> bool {
        self.endorsement_sig.len() == 64
            && !self.device_record.is_empty()
            && !self.device_record_owner.is_empty()
            && self.extends_elements.len() <= MAX_DEVICE_SET_ELEMENTS
            && self.extends_elements.windows(2).all(|w| w[0] < w[1])
            && (self.extends_elements.iter()).all(|e| e.incarnation < self.incarnation)
    }

    /// Whether it answers a fork (rule 10: then effective only with a
    /// matching `DeviceReadmit`).
    #[must_use]
    pub fn answers_fork(&self) -> bool {
        self.fork_evidence != [0; 32]
    }
}

/// `DeviceRetired{…}`: `p` retires `device_key` above `last_seq` (rules 4,
/// 5, 10). Permanent: no later endorsement revives the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRetired {
    pub device_key: [u8; 32],
    pub incarnation: u64,
    pub last_seq: u64,
    /// `p`'s Ed25519 signature; 64 bytes.
    pub retirement_sig: Vec<u8>,
}

impl DeviceRetired {
    /// Whether the body is well formed on its own: a 64-byte signature.
    #[must_use]
    pub fn well_formed(&self) -> bool {
        self.retirement_sig.len() == 64
    }
}

/// `DeviceReadmit{…}`: the op's author co-signs `member`'s `new_key` at
/// `incarnation`, answering one fork (rules 7, 8, 10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceReadmit {
    pub member: PseudonymKey,
    /// `M(p)` from the member's `MemberJoined`.
    pub member_record: String,
    pub new_key: [u8; 32],
    pub incarnation: u64,
    pub fork_evidence: [u8; 32],
}

impl DeviceReadmit {
    /// Whether the body is well formed on its own: a member record named
    /// and a fork answered (a readmit exists only after a first fork).
    #[must_use]
    pub fn well_formed(&self) -> bool {
        !self.member_record.is_empty() && self.fork_evidence != [0; 32]
    }
}
