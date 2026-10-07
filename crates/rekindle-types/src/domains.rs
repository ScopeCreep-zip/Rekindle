//! Every domain-separation label Rekindle feeds into a KDF, a signature,
//! a MAC or an AEAD nonce — in one place.
//!
//! A label scopes a key or signature to one purpose, so a value produced
//! for one protocol can never be accepted by another (cross-protocol
//! attacks; RFC 9180 §5 and the HKDF `info` guidance in RFC 5869 §3.2 make
//! the same point). That guarantee only holds if no two purposes ever
//! share a label, which is unverifiable while labels are literals spread
//! across crates. Here they are listed once, the tests below reject
//! duplicates and prefix collisions, and `cargo xtask check` rejects a
//! label-shaped literal anywhere else.
//!
//! Rules for new labels: `rekindle-<purpose>-v<N>`, lowercase, added here
//! first. Protocol-mandated labels (RFC 9605 SFrame, RFC 9420 MLS exporter
//! labels) are used verbatim by the module implementing that RFC and are not
//! listed here.
//!
//! Values are wire- and storage-significant: changing one silently breaks
//! every key, signature or record derived with it. The `PRE_FORMAT` labels
//! predate the naming rule and keep their bytes until the integration-plan
//! step noted on each removes or renames them
//! (`.claude/plans/standards-remediation/00-integration-plan.md`).

macro_rules! labels {
    ($( $(#[$doc:meta])* $name:ident = $value:literal; )*) => {
        $( $(#[$doc])* pub const $name: &str = $value; )*

        /// Every label above, by constant name.
        pub const ALL: &[(&str, &str)] = &[ $( (stringify!($name), $name) ),* ];
    };
}

labels! {
    // ── Identity, DHT records, safety numbers ────────────────────────
    /// HKDF info: account-record encryption key from the identity secret.
    ACCOUNT_KEY = "rekindle-account-v1";
    /// HKDF info prefix: per-pair conversation-record key.
    CONVERSATION_KEY = "rekindle-conversation-v1";
    /// BLAKE3 suffix: safety-number fingerprint of two identity keys.
    SAFETY_NUMBER = "rekindle-safety-v1";
    /// HKDF info: PQXDH shared secret → initial root key.
    PQXDH_ROOT = "rekindle-pqxdh-root-v1";

    // ── Signed 1:1 envelopes ─────────────────────────────────────────
    /// Ed25519 signature prefix: desktop `MessageEnvelope`, bound to the
    /// recipient's identity key.
    MSG_ENVELOPE_V2 = "rekindle-msg-envelope-v2";
    /// Ed25519 signature prefix: daemon framed `SignedPayload`, bound to
    /// the recipient's identity key and the frame `TypeId`.
    DM_FRAME_SIG_V2 = "rekindle-dm-frame-sig-v2";

    // ── Communities ──────────────────────────────────────────────────
    /// HKDF salt: per-community pseudonym from the identity secret.
    COMMUNITY_PSEUDONYM = "rekindle-community-pseudonym-v1";
    /// HKDF salt: governance overflow-page owner keypair.
    GOV_OVERFLOW = "rekindle-gov-overflow-v1";
    /// Signature prefix: governance SMPL subkey payload.
    GOV_SUBKEY = "rekindle-gov-subkey-v1";
    /// Signature prefix: channel message-record subkey payload.
    CHANNEL_SUBKEY = "rekindle-channel-subkey-v1";
    /// Signature prefix: member-registry presence row.
    PRESENCE_ROW = "rekindle-presence-v1";
    /// HKDF info: invite-secrets record key.
    INVITE_SECRETS = "rekindle-invite-secrets-v1";
    /// HKDF info: channel MEK wrapping key.
    MEK_WRAP = "rekindle-mek-wrap-v1";

    // ── 1:1 and group DMs ────────────────────────────────────────────
    /// HKDF info: group-DM MEK from the pairwise shared secret.
    DM_MEK = "rekindle-dm-mek-v1";
    /// HKDF info: group-DM MEK forward ratchet step.
    DM_MEK_RATCHET = "rekindle-dm-ratchet-v1";

    // ── Calls and voice ──────────────────────────────────────────────
    /// HKDF info: 1:1 call media key. Distinct from the friend/DM
    /// derivations so one X25519 keypair never yields the same secret in
    /// two contexts.
    CALL_KEY = "rekindle-call-key-v1";
    /// HKDF info: group-call key wrapping. Distinct from `CALL_KEY` so the
    /// same X25519 keypair in both contexts never produces the same key.
    GROUP_CALL_WRAP = "rekindle-group-call-wrap-v1";
    /// HKDF info prefix: a media sender's SFrame base key (RFC 9605 §5.1
    /// sender keys) from the scope secret, the sender's key and the
    /// sender's session tag.
    VOICE_SENDER_KEY = "rekindle-voice-sender-key-v1";
    /// HPKE info: a call-media sender key sealed to one participant
    /// (RFC 9605 §5.1 sender keys, sent pairwise; plan C7.20).
    MEDIA_KEY_SEAL = "rekindle-media-key-seal-v1";
    /// HKDF info prefix: a video sender's frame key from its media sender
    /// key and its own key, so the secret never keys two constructions.
    VIDEO_SENDER_KEY = "rekindle-video-sender-key-v1";
    /// Signature prefix: voice packet (SFrame payload, Cap'n Proto).
    VOICE_PACKET = "rekindle-voice-packet-v2";
    /// Signature prefix: voice receiver report. Distinct from
    /// `VOICE_PACKET` so a captured packet signature cannot be presented
    /// as a report signature.
    VOICE_RECEIVER_REPORT = "rekindle-voice-receiver-report-v1";

    // ── Cross-device sync and local storage ──────────────────────────
    /// HKDF salt: personal sync-record key.
    SYNC_SALT = "rekindle-sync-v1";
    /// HKDF info: device-pairing key.
    PAIRING = "rekindle-pairing-v1";
    /// BLAKE3 input: daemon session-file integrity key.
    SESSION_INTEGRITY = "rekindle-session-integrity-v1";
    /// BLAKE3 keyed-hash input: daemon keyring disk-fallback key.
    DISK_FALLBACK_KEY = "rekindle-disk-fallback-v1";

    // ── PRE_FORMAT: predate the naming rule; bytes kept until removal ──
    /// HKDF info prefix + decimal slot index: SMPL member-slot keypair.
    /// Renamed to the `-v<N>` form in step 22 (governance op DAG).
    SLOT_KEY_PREFIX = "rekindle-slot-";
    /// HKDF info: personal sync-record key (paired with `SYNC_SALT`).
    /// Replaced in step 33 (per-device sync key).
    PERSONAL_SYNC_RECORD_INFO = "personal-sync-record";
    /// HPKE info: MEK transfer — Rekindle-owned, never Veilid's
    /// `veilid-hpke/1`. Deleted with the MEK in step 30 (MLS).
    HPKE_MEK_INFO = "rekindle-mek/1";
    /// BLAKE3 `derive_key` context: vault SQLCipher key. Replaced by the
    /// vault header rework in step 15/16.
    VAULT_SQLCIPHER_KEY = "rekindle v1 vault-sqlcipher";
    /// BLAKE3 `derive_key` context: vault entry AEAD key (step 15/16).
    VAULT_ENTRY_KEY = "rekindle v1 vault-entry-gcm";
    /// 1:1 ratchet KDF labels, shared by the desktop and daemon tracks —
    /// deleted with the non-spec ratchet in step 27.
    RATCHET_PQXDH_EXPAND = "ReKindlePQXDH";
    /// See `RATCHET_PQXDH_EXPAND`.
    RATCHET_ROOT = "ReKindleRootKey";
    /// See `RATCHET_PQXDH_EXPAND`.
    RATCHET_CHAIN_RATCHET = "ReKindleChainRatchet";
    /// See `RATCHET_PQXDH_EXPAND`.
    RATCHET_MSG_KEY = "ReKindleMsgKey";
    /// See `RATCHET_PQXDH_EXPAND`.
    RATCHET_CHAIN_KEY = "ReKindleChainKey";
}

/// Labels exempt from the `rekindle-<purpose>-v<N>` rule, each scheduled
/// for removal or renaming (see the constant's doc).
pub const PRE_FORMAT: &[&str] = &[
    SLOT_KEY_PREFIX,
    PERSONAL_SYNC_RECORD_INFO,
    HPKE_MEK_INFO,
    VAULT_SQLCIPHER_KEY,
    VAULT_ENTRY_KEY,
    RATCHET_PQXDH_EXPAND,
    RATCHET_ROOT,
    RATCHET_CHAIN_RATCHET,
    RATCHET_MSG_KEY,
    RATCHET_CHAIN_KEY,
];

/// `rekindle-<purpose>-v<N>`: lowercase ASCII words joined by `-`, ending
/// in a version.
#[must_use]
pub fn is_well_formed(label: &str) -> bool {
    let Some(rest) = label.strip_prefix("rekindle-") else {
        return false;
    };
    let Some((purpose, version)) = rest.rsplit_once("-v") else {
        return false;
    };
    !purpose.is_empty()
        && purpose.split('-').all(|w| {
            !w.is_empty()
                && w.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
        && !version.is_empty()
        && version.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::{is_well_formed, ALL, PRE_FORMAT};

    #[test]
    fn every_label_is_unique() {
        for (i, (name_a, a)) in ALL.iter().enumerate() {
            for (name_b, b) in &ALL[i + 1..] {
                assert_ne!(a, b, "{name_a} and {name_b} share the label `{a}`");
            }
        }
    }

    /// Labels are often concatenated with the data they scope; if one
    /// label were a prefix of another, `A ‖ x` and `B ‖ y` could collide.
    #[test]
    fn no_label_is_a_prefix_of_another() {
        for (name_a, a) in ALL {
            for (name_b, b) in ALL {
                if name_a != name_b {
                    assert!(
                        !b.starts_with(a),
                        "{name_a} (`{a}`) is a prefix of {name_b} (`{b}`)"
                    );
                }
            }
        }
    }

    #[test]
    fn labels_follow_the_naming_rule_or_are_scheduled_for_removal() {
        for (name, label) in ALL {
            assert!(
                is_well_formed(label) || PRE_FORMAT.contains(label),
                "{name} = `{label}` must be `rekindle-<purpose>-v<N>`"
            );
            if PRE_FORMAT.contains(label) {
                assert!(
                    !is_well_formed(label),
                    "{name} is well-formed; drop it from PRE_FORMAT"
                );
            }
        }
    }

    #[test]
    fn naming_rule() {
        assert!(is_well_formed("rekindle-gov-op-v1"));
        assert!(is_well_formed("rekindle-voice-packet-v12"));
        assert!(!is_well_formed("rekindle-gov-op"));
        assert!(!is_well_formed("rekindle--v1"));
        assert!(!is_well_formed("Rekindle-x-v1"));
        assert!(!is_well_formed("rekindle-x-v"));
        assert!(!is_well_formed("rekindle v1 vault-entry-gcm"));
    }
}
