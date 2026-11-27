//! Pure per-row classification for the Plate Gate registry scan.
//!
//! Owns the W26 signature verification + ban filter + stale-heartbeat
//! / empty-route-blob classification. The adapter exposes raw bytes
//! per `(segment, subkey)` via `CommunityPresenceDeps::scan_segment_raw`;
//! the orchestrator calls [`parse_and_classify_row`] for each raw
//! payload to compute the discovered + online splits.

use std::collections::HashSet;
use std::hash::BuildHasher;

use rekindle_types::presence::MemberPresence;

use crate::deps::OnlineMember;

/// SMPL LOCAL subkeys per segment record (architecture §15.5).
/// Adapters pump 0..SUBKEYS_PER_SEGMENT through `get_dht_value`
/// when implementing `scan_segment_raw`.
///
/// Re-exported rather than redeclared: this is the same number as the
/// registry's slot count by construction — one subkey per member slot —
/// and two names for it is how the two drift. The duplicate-constant
/// gate is keyed on the name, so it could never have caught this pair.
pub use rekindle_types::dht_layout::SLOTS_PER_SEGMENT as SUBKEYS_PER_SEGMENT;

/// Outcome of one row's classification — either accepted (with the
/// MemberPresence body, hex-pseudonym, and whether it should be
/// treated as online right now) or rejected for one of the
/// documented reasons.
///
/// The `Accepted` variant carries a `MemberPresence` (~300 bytes
/// worst case with the full profile + W26 signature); the reject
/// variants are zero-sized markers. Boxed to keep the enum compact
/// and match the clippy `large_enum_variant` lint default.
#[derive(Debug, Clone, PartialEq)]
pub enum ClassifiedRow {
    /// Row passed every check. `online_member` is `Some` when the
    /// row has a fresh heartbeat and non-"offline" status (liveness
    /// only — route_blob may be empty); `None` for rows that count as
    /// "discovered" but don't enter the gossip overlay this tick.
    Accepted(Box<AcceptedRow>),
    /// Row's `MemberPresence` JSON couldn't be deserialised.
    MalformedJson,
    /// Row's signature wasn't a 64-byte buffer.
    InvalidSignatureLength,
    /// Row's signature didn't verify against its `pseudonym_key`
    /// (architecture §26 W26 — defends against forged presence
    /// writes by other slot-keypair holders).
    SignatureRejected,
    /// Row's author is in the community's ban list.
    Banned,
    /// Row's JSON payload was zero-length.
    EmptyPayload,
    /// Row is validly signed by its author, who has marked themselves
    /// departed. The slot is theirs to release and they released it.
    ///
    /// Distinct from `EmptyPayload` on purpose: this one carries a
    /// signature, so it proves *who* released the slot. An unsigned
    /// zeroing proves nothing, and under a shared slot seed anyone can
    /// write anyone's slot.
    Departed,
}

/// Payload for [`ClassifiedRow::Accepted`] — boxed inside the
/// enum so the discriminant stays small.
#[derive(Debug, Clone, PartialEq)]
pub struct AcceptedRow {
    pub pseudonym_hex: String,
    pub presence: MemberPresence,
    pub online_member: Option<OnlineMember>,
}

/// Parse + verify + classify a single registry-subkey payload.
///
/// `raw_bytes` is the bytes returned by `get_dht_value`. Empty
/// payloads (subkey unused or fetch returned nothing) yield
/// [`ClassifiedRow::EmptyPayload`].
///
/// The `online_member` slot of [`ClassifiedRow::Accepted`] applies
/// the architecture §3 gossip-overlay membership rules. Liveness is
/// decoupled from reachability — an online member may have an empty
/// `route_blob` (route not yet allocated); reachability consumers read
/// the blob off the snapshot and gate on it themselves:
/// - `status == "offline"` → not online (but still discovered for
///   member registry + role merging).
/// - `last_heartbeat <= stale_threshold` → stale; not online.
/// - otherwise → online (route_blob carried through, may be empty),
///   with `last_seen = now_secs`.
impl ClassifiedRow {
    /// The classification's name, for diagnostics.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Accepted(_) => "accepted",
            Self::MalformedJson => "malformed-json",
            Self::InvalidSignatureLength => "invalid-signature-length",
            Self::SignatureRejected => "signature-rejected",
            Self::Banned => "banned",
            Self::EmptyPayload => "empty-payload",
            Self::Departed => "departed",
        }
    }
}

#[must_use]
pub fn parse_and_classify_row<S: BuildHasher>(
    raw_bytes: &[u8],
    banned_pseudonyms: &HashSet<String, S>,
    stale_heartbeat_threshold_secs: u64,
    now_secs: u64,
) -> ClassifiedRow {
    if raw_bytes.is_empty() {
        return ClassifiedRow::EmptyPayload;
    }
    let Ok(presence) = serde_json::from_slice::<MemberPresence>(raw_bytes) else {
        return ClassifiedRow::MalformedJson;
    };
    let Ok(sig_arr) = <[u8; 64]>::try_from(presence.signature.as_slice()) else {
        return ClassifiedRow::InvalidSignatureLength;
    };
    if rekindle_secrets::derive::verify_pseudonym_signature(
        &presence.pseudonym_key.0,
        &presence.signing_bytes(),
        &sig_arr,
    )
    .is_err()
    {
        return ClassifiedRow::SignatureRejected;
    }
    // Checked only after the signature verifies: an unsigned or forged
    // "I have left" would otherwise be an eviction primitive.
    if presence.departed {
        return ClassifiedRow::Departed;
    }

    let pseudonym_hex = hex::encode(presence.pseudonym_key.0);
    if banned_pseudonyms.contains(&pseudonym_hex) {
        return ClassifiedRow::Banned;
    }

    let stale_cutoff = now_secs.saturating_sub(stale_heartbeat_threshold_secs);
    // Liveness (am I here, now) ≠ reachability (can a route reach me).
    // "Offline" is a function of status + heartbeat freshness ONLY. The
    // route blob is allocated asynchronously and is frequently empty at
    // first write; gating liveness on it made freshly-joined, actively-
    // heartbeating members invisible to every peer until their route
    // landed (seconds-to-never). Route travels in
    // `OnlineMember.route_blob` (may be empty) for the separate
    // reachability consumers (DM / MEK delivery), which check it there.
    let is_offline = presence.status == "offline" || presence.last_heartbeat <= stale_cutoff;
    let online_member = if is_offline {
        None
    } else {
        Some(OnlineMember {
            route_blob: presence.route_blob.clone(),
            status: presence.status.clone(),
            last_seen: now_secs,
            // `location` lives in the MEK-encrypted SessionExtras; the
            // orchestrator decrypts + fills it after this pure classify.
            location: presence.session.location.clone(),
            last_active: presence.session.last_active,
        })
    };

    ClassifiedRow::Accepted(Box::new(AcceptedRow {
        pseudonym_hex,
        presence,
        online_member,
    }))
}

#[cfg(test)]
#[path = "scan_row_tests.rs"]
mod tests;

/// The wire-compatibility contract for `MemberPresence::departed`.
///
/// `signing_bytes()` re-serialises the whole struct, so a field an older
/// reader does not know is dropped on the round-trip and its signature
/// fails. `skip_serializing_if` turns that from a hazard into the
/// rollout strategy, and these tests pin both halves — if either breaks,
/// deploying a departure marker starts evicting live members.
#[cfg(test)]
#[path = "scan_row_departure_wire_compat.rs"]
mod departure_wire_compat;
