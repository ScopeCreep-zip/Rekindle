//! Frozen derivation vectors — pin derivation OUTPUTS, not self-consistency.
//!
//! `pinned_derivation_stability` in originate.rs checks a == b (determinism).
//! This file checks a == "<frozen hex>" (stability across releases). A KDF
//! grammar change that is internally consistent but produces different bytes
//! passes the inline test and FAILS this one.
//!
//! To record vectors for the first time:
//!   1. Run this test — it will print the actual values to stderr.
//!   2. Copy the printed hex strings into the constants below.
//!   3. Run again — it should pass.
//!   4. Commit. The values are frozen forever.
//!
//! VERIFY BEFORE FREEZING: confirm derived values match the live PQXDH
//! convention in rekindle-chat (DH_FROM_SEED tag = "rekindle identity x25519 v1").

use rekindle_identity::originate_from_seed;
use rekindle_identity::OriginSeed;
use rekindle_identity::session_anchor;
use rekindle_identity::derive_persona;
use rekindle_identity::GovernanceKey;
use zeroize::Zeroizing;

// ── Frozen constants ────────────────────────────────────────────
//
// Each value is the hex-encoded output of the named derivation from
// the named seed. Changing ANY value is a wire-breaking change.
// If a test fails after a code change, the code change broke the
// derivation grammar — revert the code, not the constant.

/// originate_from_seed([0x01; 32]).root.to_hex()
const FROZEN_ROOT_01: &str = "8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c";

/// originate_from_seed([0x01; 32]).dh_public as hex
const FROZEN_DH_PUBLIC_01: &str = "c6ed047721d1436f32dbe3689ef3923ad9f63075752752b4c1fb1196c912a529";

/// originate_from_seed([0x01; 32]).revocation.signature as hex
const FROZEN_REVOCATION_SIG_01: &str = "f5c299d7ca34b6f3fbaaf1e09fa5955375f149adbc4ad1c231e6d85b2aa0cff83ee95c668d613709b8f8d20b996b1b7b06841023d4e8e84630bc9ac7b442c802";

/// derive_persona(seed=[0x01;32], gov="VLD0:frozen-test-community", slot=0).pseudonym.to_hex()
const FROZEN_PSEUDONYM_01: &str = "40a1b16ff37f8034b4e7540f7d6bbc347eb08412cfd34ad5337c57a5af6be0dc";

/// session_anchor(root_from_seed(0x02), root_from_seed(0x03)).to_hex()
const FROZEN_SESSION_ANCHOR_02_03: &str = "09a06bd872902bf7af80d74e34304387e59ab07dcca8bf3efd58c0129f1180c8";

// ── Tests ───────────────────────────────────────────────────────

#[test]
fn frozen_originate_root() {
    let o = originate_from_seed(
        OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]))
    ).unwrap();

    let actual = o.root.to_hex();
    eprintln!("FROZEN_ROOT_01 = \"{actual}\"");

    assert_eq!(
        actual, FROZEN_ROOT_01,
        "Root derivation changed — this is a wire-breaking grammar change. \
         If intentional, update the frozen constant. If not, revert."
    );
}

#[test]
fn frozen_originate_dh_public() {
    let o = originate_from_seed(
        OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]))
    ).unwrap();

    let actual = hex::encode(o.dh_public);
    eprintln!("FROZEN_DH_PUBLIC_01 = \"{actual}\"");

    assert_eq!(
        actual, FROZEN_DH_PUBLIC_01,
        "DH public derivation changed — wire break."
    );
}

#[test]
fn frozen_revocation_signature() {
    let o = originate_from_seed(
        OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]))
    ).unwrap();

    let actual = hex::encode(o.revocation.signature.as_bytes());
    eprintln!("FROZEN_REVOCATION_SIG_01 = \"{actual}\"");

    assert_eq!(
        actual, FROZEN_REVOCATION_SIG_01,
        "Revocation signature changed — wire break."
    );
}

#[test]
fn frozen_pseudonym_derivation() {
    let seed = OriginSeed::from_vault_bytes(Zeroizing::new([0x01; 32]));
    let gov = GovernanceKey::parse("VLD0:frozen-test-community").unwrap();
    let (persona, _) = derive_persona(&seed, &gov, 0).unwrap();

    let actual = persona.pseudonym.to_hex();
    eprintln!("FROZEN_PSEUDONYM_01 = \"{actual}\"");

    assert_eq!(
        actual, FROZEN_PSEUDONYM_01,
        "Pseudonym derivation changed — wire break."
    );
}

#[test]
fn frozen_session_anchor() {
    let o2 = originate_from_seed(
        OriginSeed::from_vault_bytes(Zeroizing::new([0x02; 32]))
    ).unwrap();
    let o3 = originate_from_seed(
        OriginSeed::from_vault_bytes(Zeroizing::new([0x03; 32]))
    ).unwrap();

    let anchor = session_anchor(&o2.root, &o3.root).unwrap();
    let actual = anchor.to_hex();
    eprintln!("FROZEN_SESSION_ANCHOR_02_03 = \"{actual}\"");

    assert_eq!(
        actual, FROZEN_SESSION_ANCHOR_02_03,
        "Session anchor derivation changed — wire break."
    );
}
