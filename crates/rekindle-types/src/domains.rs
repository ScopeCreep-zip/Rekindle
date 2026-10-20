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
    // ── Identity and DHT records ────────────────────────────────
    /// HKDF info: account-record encryption key from the identity secret.
    ACCOUNT_KEY = "rekindle-account-v1";
    /// HKDF info: the PQXDH key material (X25519, SHA-256, ML-KEM-768, with
    /// EncodeEC/EncodeKEM of the bound keys) → `SKec ‖ SKscka` (plan E2.2).
    PQXDH_SK = "rekindle-pqxdh-sk-v1";

    // ── 1:1 frames (plan E2.3) ───────────────────────────────────────
    /// Ed25519 signature prefix: a pre-session frame, over
    /// `recipient ‖ typeId ‖ enc ‖ ct`.
    DM_PRESESSION_SIG = "rekindle-dm-presession-sig-v1";
    /// HPKE info: a sealed `FriendRequest` (0x0002).
    DM_FRIEND_REQUEST_HPKE = "rekindle-dm-friend-request-hpke-v1";
    /// HPKE info: a sealed `FriendReject` (0x0102).
    DM_FRIEND_REJECT_HPKE = "rekindle-dm-friend-reject-hpke-v1";
    /// HPKE info: a sealed `PreKeyRejected` (0x0005).
    DM_PREKEY_REJECTED_HPKE = "rekindle-dm-prekey-rejected-hpke-v1";
    /// BLAKE3 derive-key context: the initiator base key a `PreKeyRejected`
    /// names.
    PREKEY_REJECTED_BASE_KEY = "rekindle-prekey-rejected-base-key-v1";
    /// HPKE info: a sealed `SessionResetRequest` (0x0007).
    DM_SESSION_RESET_REQUEST_HPKE = "rekindle-dm-session-reset-request-hpke-v1";
    /// HPKE info: a sealed `SessionResetDecline` (0x0009).
    DM_SESSION_RESET_DECLINE_HPKE = "rekindle-dm-session-reset-decline-hpke-v1";
    /// HPKE info: a sealed `RegisterPushRelay` (0x0310).
    DM_PUSH_REGISTER_HPKE = "rekindle-dm-push-register-hpke-v1";
    /// HPKE info: a sealed `UnregisterPushRelay` (0x0311).
    DM_PUSH_UNREGISTER_HPKE = "rekindle-dm-push-unregister-hpke-v1";
    /// HPKE info: a sealed `WakeNotify` (0x0312).
    DM_PUSH_WAKE_HPKE = "rekindle-dm-push-wake-hpke-v1";
    /// Ed25519 signature prefix: an identity rotation, signed by the old and
    /// the new identity over `old ‖ new ‖ recipient` (0x0108).
    IDENTITY_ROTATED = "rekindle-identity-rotated-v1";

    // ── Prekey record and invites (plan E2.4; audit 2026-10-20 C-2) ──
    /// Ed25519 signature prefix: the signed prekey, by the identity, over
    /// `u32be(spk_id) ‖ i64be(issued_at) ‖ EncodeEC(spk)`.
    PREKEY_SPK_SIG = "rekindle-prekey-spk-sig-v1";
    /// Ed25519 signature prefix: the ML-KEM last-resort prekey, by the
    /// identity, over `u32be(id) ‖ i64be(issued_at) ‖ EncodeKEM(pqpk)`.
    PREKEY_PQ_LR_SIG = "rekindle-prekey-pq-lr-sig-v1";
    /// Ed25519 signature prefix: a one-time ML-KEM prekey, by the identity,
    /// over `u32be(id) ‖ EncodeKEM(pqpk)`.
    PREKEY_PQ_OT_SIG = "rekindle-prekey-pq-ot-sig-v1";
    /// Ed25519 signature prefix: a one-time curve prekey, by the identity,
    /// over `u32be(opk_id) ‖ EncodeEC(opk)`.
    PREKEY_OPK_SIG = "rekindle-prekey-opk-sig-v1";
    /// Ed25519 signature prefix: a friend invite blob, by the identity,
    /// over the JSON of every field but the signature.
    FRIEND_INVITE_SIG = "rekindle-friend-invite-sig-v1";

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
    /// Ed25519 signature prefix: the daemon's framed community RPC
    /// (`SignedPayload`), bound to the recipient's identity key and the
    /// frame `TypeId`. v3 dropped `seq` and the correlation id with the
    /// 1:1 frames (plan E2.3); v2 (`rekindle-dm-frame-sig-v2`) is retired.
    FRAME_SIG_V3 = "rekindle-frame-sig-v3";

    // ── 1:1 and group DMs ────────────────────────────────────────────
    /// HKDF info: group-DM MEK forward ratchet step.
    DM_MEK_RATCHET = "rekindle-dm-ratchet-v1";
    /// HKDF info: the spec Double Ratchet's `KDF_RK` (DR rev. 4 §7.2).
    DR_ROOT = "rekindle-dr-root-v1";
    /// HKDF info: a Double Ratchet message key → the 80 bytes of DR rev. 4
    /// §7.2's `ENCRYPT`: AES-256-CBC key, HMAC-SHA256 key and IV. v1 (an
    /// AES-256-GCM key and nonce) is retired.
    DR_MSG = "rekindle-dr-msg-v2";

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
    /// HPKE info: a call-media sender key sealed to one participant's
    /// per-session key (RFC 9605 §5.1 sender keys, sent pairwise; plans
    /// C7.20, C7.29). v2: HPKE Base to the session key, not Auth between
    /// pseudonyms.
    MEDIA_KEY_SEAL = "rekindle-media-key-seal-v2";
    /// Signature prefix: a participant's per-session media key, signed by
    /// its community pseudonym so a roster can relay it (plan C7.29).
    MEDIA_SESSION_KEY = "rekindle-media-session-key-v1";
    /// Signature prefix: a sealed call-media key, signed by its sender's
    /// pseudonym over channel, recipient, index and the sealed bytes
    /// (plan C7.29).
    MEDIA_KEY_SIG = "rekindle-media-key-sig-v1";
    /// HKDF info prefix: a video sender's frame key from its media sender
    /// key and its own key, so the secret never keys two constructions.
    VIDEO_SENDER_KEY = "rekindle-video-sender-key-v1";
    /// Signature prefix: voice packet (SFrame payload, Cap'n Proto).
    VOICE_PACKET = "rekindle-voice-packet-v3";
    /// Signature prefix: voice receiver report. Distinct from
    /// `VOICE_PACKET` so a captured packet signature cannot be presented
    /// as a report signature.
    VOICE_RECEIVER_REPORT = "rekindle-voice-receiver-report-v1";
    /// Signature prefix: a padding datagram on a media route (plan
    /// E4.3.3), binding the sender to the route sequence number it fills.
    MEDIA_PADDING = "rekindle-media-padding-v1";
    /// Signature prefix: transport feedback for the per-route bandwidth
    /// estimator (plan E4.3.2). Distinct from the media and report
    /// prefixes so no other signature can be presented as feedback.
    TRANSPORT_FEEDBACK = "rekindle-transport-feedback-v1";

    // ── Cross-device sync and local storage ──────────────────────────
    /// HKDF salt: personal sync-record key.
    SYNC_SALT = "rekindle-sync-v1";
    /// HKDF info: device-pairing key.
    PAIRING = "rekindle-pairing-v1";

    /// BLAKE3 `derive_key` context: the vault's SQLCipher key (plan D1).
    VAULT_SQLCIPHER_KEY = "rekindle-vault-sqlcipher-v1";
    /// BLAKE3 `derive_key` context: the vault's per-entry AEAD key.
    VAULT_ENTRY_KEY = "rekindle-vault-entry-v1";
    /// BLAKE3 `derive_key` context: the identity database's SQLCipher key,
    /// derived from the vault passphrase (plan D1).
    VAULT_APP_DB_KEY = "rekindle-vault-app-db-v1";
    /// HKDF info (salt none): the key that names an identity's file-cache
    /// directories (keyed BLAKE3 of each scope and attachment), from the
    /// identity secret (plan E3.0a).
    FILE_CACHE_NAMES = "rekindle-file-cache-names-v1";
    /// Keyed-BLAKE3 input prefix, then `:` and the id: a community's
    /// attachment cache directory name.
    FILE_CACHE_COMMUNITY = "rekindle-file-cache-community-v1";
    /// Keyed-BLAKE3 input prefix, then `:` and the id: a conversation's
    /// attachment cache directory name.
    FILE_CACHE_DM = "rekindle-file-cache-dm-v1";
    /// Keyed-BLAKE3 input prefix, then `:` and the id: a community's avatar
    /// and banner cache directory name.
    FILE_CACHE_AVATARS = "rekindle-file-cache-avatars-v1";
    /// Keyed-BLAKE3 input prefix, then `:`, the scope's directory name, `:`
    /// and the attachment id: an attachment's directory name.
    FILE_CACHE_ATTACHMENT = "rekindle-file-cache-attachment-v1";
    // ── PRE_FORMAT: predate the naming rule; bytes kept until removal ──
    /// HKDF info prefix + decimal slot index: SMPL member-slot keypair.
    /// Renamed to the `-v<N>` form in step 22 (governance op DAG).
    SLOT_KEY_PREFIX = "rekindle-slot-";
    /// HKDF info: personal sync-record key (paired with `SYNC_SALT`).
    /// Replaced in step 33 (per-device sync key).
    PERSONAL_SYNC_RECORD_INFO = "personal-sync-record";
    /// HPKE info: MEK transfer — Rekindle-owned, never Veilid's
    /// `veilid-hpke/1`. Renamed by E3.5's signed v2 wrap.
    HPKE_MEK_INFO = "rekindle-mek/1";
}

/// Every prefix an identity key (Ed25519) signs under. One key signs all
/// of them, so none may be a prefix of another: a signature for one
/// purpose then never verifies for another.
pub const IDENTITY_KEY_SIGNATURES: &[&str] = &[
    DM_PRESESSION_SIG,
    IDENTITY_ROTATED,
    FRAME_SIG_V3,
    PREKEY_SPK_SIG,
    PREKEY_PQ_LR_SIG,
    PREKEY_PQ_OT_SIG,
    PREKEY_OPK_SIG,
    FRIEND_INVITE_SIG,
];

/// Labels exempt from the `rekindle-<purpose>-v<N>` rule, each scheduled
/// for removal or renaming (see the constant's doc).
pub const PRE_FORMAT: &[&str] = &[SLOT_KEY_PREFIX, PERSONAL_SYNC_RECORD_INFO, HPKE_MEK_INFO];

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
    use super::{is_well_formed, ALL, IDENTITY_KEY_SIGNATURES, PRE_FORMAT};

    /// The identity key signs under each of these and nothing else; every
    /// one is distinct, none is a prefix of another, and each is in the
    /// registry.
    #[test]
    fn identity_key_signing_domains_are_distinct_and_prefix_free() {
        for (i, a) in IDENTITY_KEY_SIGNATURES.iter().enumerate() {
            assert!(ALL.iter().any(|(_, l)| l == a), "`{a}` is not registered");
            for (j, b) in IDENTITY_KEY_SIGNATURES.iter().enumerate() {
                if i != j {
                    assert!(!b.starts_with(a), "`{a}` is a prefix of `{b}`");
                }
            }
        }
    }

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
