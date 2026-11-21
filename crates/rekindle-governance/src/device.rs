//! The signed layouts and hashes of ADR 0022 rule 10 (plan E3.2
//! S7d-keys.1): a pseudonym's device-key endorsement and retirement, a
//! `DeviceReadmit`'s body, the endorsement set's `extendsSet`, fork evidence
//! and the "retired by restore" marker. Each text field is prefixed by its
//! length (u32, big-endian) and each integer is big-endian, so no two field
//! tuples share bytes. Judging an op's key at its deps is S7d-keys.2.

use rekindle_secrets::sign::verify_signature;
use rekindle_types::domains;
use rekindle_types::governance::{
    DeviceEndorsement, DeviceReadmit, DeviceRetired, DeviceSetElement, DeviceSetElementKind, OpHash,
};
use rekindle_types::id::PseudonymKey;

fn text(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&u32::try_from(s.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn labelled(label: &str, community: &str, member_record: &str) -> Vec<u8> {
    let mut out = label.as_bytes().to_vec();
    text(&mut out, community);
    text(&mut out, member_record);
    out
}

/// `extendsSet`: BLAKE3 `derive_key(GOV_DEVICE_SET)` over the count and
/// the sorted element ids `(kind, device key, incarnation)`, each once.
#[must_use]
pub fn device_set_hash(elements: &[DeviceSetElement]) -> [u8; 32] {
    let mut sorted = elements.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut h = blake3::Hasher::new_derive_key(domains::GOV_DEVICE_SET);
    h.update(
        &u32::try_from(sorted.len())
            .unwrap_or(u32::MAX)
            .to_be_bytes(),
    );
    for e in sorted {
        h.update(&[match e.kind {
            DeviceSetElementKind::Endorsement => 0,
            DeviceSetElementKind::Retirement => 1,
        }]);
        h.update(&e.device_key);
        h.update(&e.incarnation.to_be_bytes());
    }
    *h.finalize().as_bytes()
}

/// The bytes `p` signs to endorse `e` in `community` for member record
/// `member_record` (rule 10): `GOV_DEVICE_KEY ‖ community ‖ memberRecord ‖
/// deviceKey ‖ incarnation ‖ extendsSet ‖ deviceRecord ‖ deviceRecordOwner ‖
/// forkEvidence`.
#[must_use]
pub fn endorsement_signing_bytes(
    community: &str,
    member_record: &str,
    e: &DeviceEndorsement,
) -> Vec<u8> {
    let mut out = labelled(domains::GOV_DEVICE_KEY, community, member_record);
    out.extend_from_slice(&e.device_key);
    out.extend_from_slice(&e.incarnation.to_be_bytes());
    out.extend_from_slice(&e.extends_set);
    text(&mut out, &e.device_record);
    text(&mut out, &e.device_record_owner);
    out.extend_from_slice(&e.fork_evidence);
    out
}

/// Whether `e` is well formed on its own and its `extendsSet` is the hash
/// of the element ids it carries: what a reader checks before it holds the
/// community context (`validate_write`).
#[must_use]
pub fn endorsement_well_formed(e: &DeviceEndorsement) -> bool {
    e.well_formed() && device_set_hash(&e.extends_elements) == e.extends_set
}

/// Whether `e` is `p`'s endorsement for `community` and `member_record`:
/// well formed, and its signature verifies over the rule-10 layout.
#[must_use]
pub fn endorsement_holds(
    p: &PseudonymKey,
    community: &str,
    member_record: &str,
    e: &DeviceEndorsement,
) -> bool {
    let Ok(sig) = <[u8; 64]>::try_from(e.endorsement_sig.as_slice()) else {
        return false;
    };
    endorsement_well_formed(e)
        && verify_signature(
            &p.0,
            &endorsement_signing_bytes(community, member_record, e),
            &sig,
        )
        .is_ok()
}

/// The bytes `p` signs to retire a key (rule 10): `GOV_DEVICE_RETIRED ‖
/// community ‖ memberRecord ‖ deviceKey ‖ incarnation ‖ lastSeq`.
#[must_use]
pub fn retirement_signing_bytes(
    community: &str,
    member_record: &str,
    r: &DeviceRetired,
) -> Vec<u8> {
    let mut out = labelled(domains::GOV_DEVICE_RETIRED, community, member_record);
    out.extend_from_slice(&r.device_key);
    out.extend_from_slice(&r.incarnation.to_be_bytes());
    out.extend_from_slice(&r.last_seq.to_be_bytes());
    out
}

/// Whether `r` is `p`'s retirement for `community` and `member_record`.
#[must_use]
pub fn retirement_holds(
    p: &PseudonymKey,
    community: &str,
    member_record: &str,
    r: &DeviceRetired,
) -> bool {
    let Ok(sig) = <[u8; 64]>::try_from(r.retirement_sig.as_slice()) else {
        return false;
    };
    verify_signature(
        &p.0,
        &retirement_signing_bytes(community, member_record, r),
        &sig,
    )
    .is_ok()
}

/// A `DeviceReadmit`'s body (rule 10): `GOV_DEVICE_READMIT ‖ community ‖
/// memberRecord ‖ newKey ‖ incarnation ‖ forkEvidence`, so a co-signature
/// answers one fork and one incarnation.
#[must_use]
pub fn readmit_body_bytes(community: &str, r: &DeviceReadmit) -> Vec<u8> {
    let mut out = labelled(domains::GOV_DEVICE_READMIT, community, &r.member_record);
    out.extend_from_slice(&r.new_key);
    out.extend_from_slice(&r.incarnation.to_be_bytes());
    out.extend_from_slice(&r.fork_evidence);
    out
}

/// Fork evidence's hash (rule 5): BLAKE3 `derive_key(GOV_FORK_EVIDENCE)`
/// over `community ‖ device key ‖ seq ‖` the two op hashes in byte order.
#[must_use]
pub fn fork_evidence_hash(
    community: &str,
    device_key: &[u8; 32],
    seq: u64,
    ops: [OpHash; 2],
) -> [u8; 32] {
    let (lo, hi) = if ops[0].0 <= ops[1].0 {
        (ops[0], ops[1])
    } else {
        (ops[1], ops[0])
    };
    let mut h = blake3::Hasher::new_derive_key(domains::GOV_FORK_EVIDENCE);
    let mut c = Vec::new();
    text(&mut c, community);
    h.update(&c);
    h.update(device_key);
    h.update(&seq.to_be_bytes());
    h.update(&lo.0);
    h.update(&hi.0);
    *h.finalize().as_bytes()
}

/// The "retired by restore" marker (rule 8): BLAKE3
/// `derive_key(GOV_RESTORE_EVIDENCE)` over `community ‖ device key ‖
/// incarnation ‖ lastSeq ‖` the published `GovHead` op hash the barrier read.
#[must_use]
pub fn restore_evidence_hash(community: &str, r: &DeviceRetired, gov_head: &OpHash) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key(domains::GOV_RESTORE_EVIDENCE);
    let mut c = Vec::new();
    text(&mut c, community);
    h.update(&c);
    h.update(&r.device_key);
    h.update(&r.incarnation.to_be_bytes());
    h.update(&r.last_seq.to_be_bytes());
    h.update(&gov_head.0);
    *h.finalize().as_bytes()
}

#[cfg(test)]
#[path = "device_tests.rs"]
mod tests;
