//! Cap'n Proto encoder / decoder for the community gossip envelope
//! (architecture §29 / `.claude/plans/community-envelope-capnp-migration.md`).
//!
//! This module is the wire-format implementation that replaces the
//! pre-migration JSON encoding. Public API:
//!
//! - `encode_signed_envelope` / `decode_signed_envelope` — outer wrapper
//!   carrying the Ed25519 signature, TTL, sender pseudonym, and inner
//!   payload bytes.
//! - `encode_community_envelope` / `decode_community_envelope` — typed
//!   inner payload (5-arm union, 67 nested control variants).
//! - `try_decode_community_envelope` — forward-compat helper. Returns
//!   `Ok(None)` for unknown variants so a relay can verify the
//!   signature, decrement TTL, and forward the bytes intact without
//!   dispatching the unknown payload locally.
//! - `encode_governance_entry` / `decode_governance_entry` — used by
//!   bootstrap responses and any future direct serialization of a
//!   single governance entry.
//!
//! Forward-compat policy mirrors libp2p gossipsub / Briar BSP / Matrix
//! federation: unknown union discriminants → drop locally + forward.
//! Truncation / malformed bytes in known variants → drop, do not
//! forward.

use crate::capnp_codec::{bounds, capnp_err, not_in_schema, pack, text_to_string, unpack_bounded};
use crate::community::envelope::{CommunityEnvelope, SignedEnvelope};
use crate::error::CodecError;

mod control;
mod governance;
mod sub_types;

/// Convert a `usize` length to a Cap'n Proto-friendly `u32`. Cap'n
/// Proto lists are bounded by `u32::MAX`; any caller exceeding that is
/// a real bug, not a silent truncation case.
pub(super) fn len_u32(n: usize) -> u32 {
    u32::try_from(n).expect("Cap'n Proto list length must fit in u32")
}

pub use control::{decode_control_payload, encode_control_payload};
pub use governance::{decode_governance_entry, encode_governance_entry};
pub(crate) use governance::{read_governance_entry, write_governance_entry};

/// Encode a `CommunityEnvelope` into packed Cap'n Proto bytes.
pub fn encode_community_envelope(env: &CommunityEnvelope) -> Result<Vec<u8>, CodecError> {
    let mut builder = capnp::message::Builder::new_default();
    let root =
        builder.init_root::<crate::community_envelope_capnp::community_envelope::Builder<'_>>();
    write_community_envelope(root, env)?;
    Ok(pack(&builder))
}

/// Decode a `CommunityEnvelope` from packed Cap'n Proto bytes.
///
/// Errors on truncation / malformed bytes; for known schemas with
/// unknown variants use `try_decode_community_envelope`.
pub fn decode_community_envelope(bytes: &[u8]) -> Result<CommunityEnvelope, CodecError> {
    let reader = unpack_bounded(bytes, bounds::COMMUNITY_ENVELOPE_BOUND)?;
    let root = reader
        .get_root::<crate::community_envelope_capnp::community_envelope::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    read_community_envelope(root)
}

/// Forward-compat decode: returns `Ok(None)` for unknown union
/// discriminants instead of an error. Callers that act as gossip
/// relays use this so they can still verify the signature, decrement
/// TTL, and forward bytes intact even when they don't understand the
/// inner variant. Truncation / malformed bytes still return `Err`.
pub fn try_decode_community_envelope(
    bytes: &[u8],
) -> Result<Option<CommunityEnvelope>, CodecError> {
    match decode_community_envelope(bytes) {
        Ok(env) => Ok(Some(env)),
        Err(CodecError::UnknownVariant(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Encode a `SignedEnvelope` into packed Cap'n Proto bytes. The inner
/// `payload` field is treated as opaque — callers must already have
/// encoded the inner `CommunityEnvelope` via `encode_community_envelope`.
pub fn encode_signed_envelope(signed: &SignedEnvelope) -> Vec<u8> {
    let mut builder = capnp::message::Builder::new_default();
    let mut root =
        builder.init_root::<crate::community_envelope_capnp::signed_envelope::Builder<'_>>();
    root.set_community_id(&signed.community_id);
    root.set_sender_pseudonym(&signed.sender_pseudonym);
    root.set_payload(&signed.envelope_bytes);
    root.set_signature(&signed.signature);
    root.set_ttl(signed.ttl);
    root.set_message_id(&signed.message_id);
    pack(&builder)
}

/// Decode a `SignedEnvelope` from packed Cap'n Proto bytes.
pub fn decode_signed_envelope(bytes: &[u8]) -> Result<SignedEnvelope, CodecError> {
    let reader = unpack_bounded(bytes, bounds::SIGNED_ENVELOPE_BOUND)?;
    let root = reader
        .get_root::<crate::community_envelope_capnp::signed_envelope::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    // A missing or wrong-length id is refused, never defaulted: a
    // defaulted id would make every such envelope one message to the
    // dedup cache.
    let message_id: [u8; crate::envelope::MESSAGE_ID_LEN] = root
        .get_message_id()
        .map_err(|e| capnp_err(&e))?
        .try_into()
        .map_err(|_| CodecError::Deserialization("gossip message id must be 16 bytes".into()))?;
    Ok(SignedEnvelope {
        community_id: text_to_string(root.get_community_id().map_err(|e| capnp_err(&e))?)?,
        sender_pseudonym: text_to_string(root.get_sender_pseudonym().map_err(|e| capnp_err(&e))?)?,
        message_id,
        envelope_bytes: root.get_payload().map_err(|e| capnp_err(&e))?.to_vec(),
        signature: root.get_signature().map_err(|e| capnp_err(&e))?.to_vec(),
        ttl: root.get_ttl(),
    })
}

// ── Inner CommunityEnvelope writer / reader ──────────────────────────

fn write_community_envelope(
    builder: crate::community_envelope_capnp::community_envelope::Builder<'_>,
    env: &CommunityEnvelope,
) -> Result<(), CodecError> {
    match env {
        CommunityEnvelope::Control(payload) => {
            let ctrl = builder.init_control();
            encode_control_payload(ctrl, payload)?;
        }
        CommunityEnvelope::TypingIndicator {
            channel_tag,
            ttl_ms,
            pseudonym_key,
        } => {
            let mut t = builder.init_typing_indicator();
            t.set_channel_tag(channel_tag);
            t.set_ttl_ms(*ttl_ms);
            t.set_pseudonym_key(pseudonym_key);
        }
        CommunityEnvelope::WatchRelay {
            record_key,
            subkey,
            content_hash,
            observer_pseudonym,
        } => {
            let mut w = builder.init_watch_relay();
            w.set_record_key(record_key);
            w.set_subkey(*subkey);
            w.set_content_hash(content_hash);
            w.set_observer_pseudonym(observer_pseudonym);
        }
    }
    Ok(())
}

fn read_community_envelope(
    reader: crate::community_envelope_capnp::community_envelope::Reader<'_>,
) -> Result<CommunityEnvelope, CodecError> {
    use crate::community_envelope_capnp::community_envelope::Which;
    match reader.which().map_err(not_in_schema)? {
        // @0 was `messageNotification` (plan E3.3 slice 8) and @2
        // `presenceUpdate` (plan E3.4 S5): no member sends either, and both
        // are skipped like an unknown arm, never acted on.
        Which::Obsolete0(()) | Which::Obsolete2(()) => Err(CodecError::UnknownVariant(
            "obsolete community envelope".to_string(),
        )),
        Which::Control(c) => {
            let c = c.map_err(|e| capnp_err(&e))?;
            Ok(CommunityEnvelope::control(decode_control_payload(c)?))
        }
        Which::TypingIndicator(t) => {
            let t = t.map_err(|e| capnp_err(&e))?;
            let channel_tag = (t.get_channel_tag().map_err(|e| capnp_err(&e))?)
                .try_into()
                .map_err(|_| {
                    CodecError::Deserialization("typing channel tag is not 16 bytes".into())
                })?;
            Ok(CommunityEnvelope::TypingIndicator {
                channel_tag,
                ttl_ms: t.get_ttl_ms(),
                pseudonym_key: text_to_string(t.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
            })
        }
        Which::WatchRelay(w) => {
            let w = w.map_err(|e| capnp_err(&e))?;
            Ok(CommunityEnvelope::WatchRelay {
                record_key: text_to_string(w.get_record_key().map_err(|e| capnp_err(&e))?)?,
                subkey: w.get_subkey(),
                content_hash: text_to_string(w.get_content_hash().map_err(|e| capnp_err(&e))?)?,
                observer_pseudonym: text_to_string(
                    w.get_observer_pseudonym().map_err(|e| capnp_err(&e))?,
                )?,
            })
        }
    }
}

#[cfg(test)]
#[path = "retired_control_tests.rs"]
mod retired_control_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_envelope_roundtrip() {
        let signed = SignedEnvelope {
            message_id: [7u8; 16],
            community_id: "VLD0:abc".into(),
            sender_pseudonym: "deadbeef".into(),
            envelope_bytes: vec![1, 2, 3, 4, 5],
            signature: vec![0u8; 64],
            ttl: 5,
        };
        let bytes = encode_signed_envelope(&signed);
        let back = decode_signed_envelope(&bytes).unwrap();
        assert_eq!(signed.community_id, back.community_id);
        assert_eq!(signed.sender_pseudonym, back.sender_pseudonym);
        assert_eq!(signed.envelope_bytes, back.envelope_bytes);
        assert_eq!(signed.signature, back.signature);
        assert_eq!(signed.ttl, back.ttl);
        assert_eq!(signed.message_id, back.message_id);
    }

    #[test]
    fn typing_indicator_roundtrip() {
        let env = CommunityEnvelope::TypingIndicator {
            channel_tag: [7; 16],
            ttl_ms: 6_000,
            pseudonym_key: "abcd".into(),
        };
        let bytes = encode_community_envelope(&env).unwrap();
        let back = decode_community_envelope(&bytes).unwrap();
        match back {
            CommunityEnvelope::TypingIndicator {
                channel_tag,
                ttl_ms,
                pseudonym_key,
            } => {
                assert_eq!(channel_tag, [7; 16]);
                assert_eq!(ttl_ms, 6_000);
                assert_eq!(pseudonym_key, "abcd");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn voice_join_handshake_roundtrip() {
        use crate::community::envelope::{ControlPayload, MediaSessionKey};

        // Leg 1 — VoiceJoin with carried display name.
        let env = CommunityEnvelope::control(ControlPayload::VoiceJoin {
            channel_id: "ch_voice".into(),
            route_blob: vec![9, 9, 9],
            display_name: Some("FireStarter92".into()),
            session_key: MediaSessionKey {
                key: vec![5; 32],
                signature: vec![6; 64],
            },
        });
        let bytes = encode_community_envelope(&env).unwrap();
        match decode_community_envelope(&bytes).unwrap().into_control() {
            Some(ControlPayload::VoiceJoin {
                channel_id,
                route_blob,
                display_name,
                session_key,
            }) => {
                assert_eq!(channel_id, "ch_voice");
                assert_eq!(route_blob, vec![9, 9, 9]);
                assert_eq!(display_name.as_deref(), Some("FireStarter92"));
                assert_eq!(session_key.key, vec![5; 32]);
                assert_eq!(session_key.signature, vec![6; 64]);
            }
            _ => panic!("wrong variant"),
        }

        // Leg 2 — VoiceJoinAck carries acker identity + route.
        let env = CommunityEnvelope::control(ControlPayload::VoiceJoinAck {
            channel_id: "ch_voice".into(),
            joiner_pseudonym: "joiner_hex".into(),
            display_name: None,
            route_blob: vec![4, 2],
            session_key: MediaSessionKey {
                key: vec![7; 32],
                signature: vec![8; 64],
            },
        });
        let bytes = encode_community_envelope(&env).unwrap();
        match decode_community_envelope(&bytes).unwrap().into_control() {
            Some(ControlPayload::VoiceJoinAck {
                channel_id,
                joiner_pseudonym,
                display_name,
                route_blob,
                session_key,
            }) => {
                assert_eq!(channel_id, "ch_voice");
                assert_eq!(joiner_pseudonym, "joiner_hex");
                assert_eq!(display_name, None, "empty wire text decodes to None");
                assert_eq!(route_blob, vec![4, 2]);
                assert_eq!(session_key.key, vec![7; 32]);
                assert_eq!(session_key.signature, vec![8; 64]);
            }
            _ => panic!("wrong variant"),
        }

        // Leg 3 — VoiceJoinConfirmed.
        let env = CommunityEnvelope::control(ControlPayload::VoiceJoinConfirmed {
            channel_id: "ch_voice".into(),
        });
        let bytes = encode_community_envelope(&env).unwrap();
        match decode_community_envelope(&bytes).unwrap().into_control() {
            Some(ControlPayload::VoiceJoinConfirmed { channel_id }) => {
                assert_eq!(channel_id, "ch_voice");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn voice_roster_entry_display_name_roundtrip() {
        use crate::community::envelope::{ControlPayload, MediaSessionKey, VoiceRosterEntry};

        let env = CommunityEnvelope::control(ControlPayload::VoiceRoster {
            channel_id: "ch_voice".into(),
            participants: vec![
                VoiceRosterEntry {
                    pseudonym_key: "alice_hex".into(),
                    route_blob: vec![1],
                    muted: true,
                    deafened: false,
                    display_name: Some("Alice".into()),
                    session_key: MediaSessionKey {
                        key: vec![3; 32],
                        signature: vec![4; 64],
                    },
                },
                VoiceRosterEntry {
                    pseudonym_key: "bob_hex".into(),
                    route_blob: vec![2],
                    muted: false,
                    deafened: false,
                    display_name: None,
                    session_key: MediaSessionKey::default(),
                },
            ],
        });
        let bytes = encode_community_envelope(&env).unwrap();
        match decode_community_envelope(&bytes).unwrap().into_control() {
            Some(ControlPayload::VoiceRoster { participants, .. }) => {
                assert_eq!(participants.len(), 2);
                assert_eq!(participants[0].display_name.as_deref(), Some("Alice"));
                assert!(participants[0].muted);
                assert_eq!(participants[1].display_name, None);
                assert_eq!(participants[0].session_key.key, vec![3; 32]);
                assert_eq!(participants[0].session_key.signature, vec![4; 64]);
                assert_eq!(participants[1].session_key, MediaSessionKey::default());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn watch_relay_roundtrip() {
        let env = CommunityEnvelope::WatchRelay {
            record_key: "VLD0:xyz".into(),
            subkey: 12,
            content_hash: "blake3:abc".into(),
            observer_pseudonym: "obsvr".into(),
        };
        let bytes = encode_community_envelope(&env).unwrap();
        let back = decode_community_envelope(&bytes).unwrap();
        match back {
            CommunityEnvelope::WatchRelay {
                record_key,
                subkey,
                content_hash,
                observer_pseudonym,
            } => {
                assert_eq!(record_key, "VLD0:xyz");
                assert_eq!(subkey, 12);
                assert_eq!(content_hash, "blake3:abc");
                assert_eq!(observer_pseudonym, "obsvr");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn try_decode_returns_ok_for_known_variant() {
        let env = CommunityEnvelope::TypingIndicator {
            channel_tag: [1; 16],
            ttl_ms: 0,
            pseudonym_key: "abc".into(),
        };
        let bytes = encode_community_envelope(&env).unwrap();
        let back = try_decode_community_envelope(&bytes).unwrap();
        assert!(back.is_some());
    }

    /// Forward-compat contract: malformed bytes return `Err`, distinct
    /// from the `Ok(None)` returned by `try_decode_community_envelope`
    /// when a known-schema reader sees an unknown union discriminant
    /// (i.e. a peer running a newer build that added a future
    /// variant). Gossip relays use the latter to forward bytes intact;
    /// the former drops + does not forward. Synthesising an actual
    /// unknown-discriminant message requires low-level Cap'n Proto
    /// wire-format construction; that path is covered by the integration
    /// path in `services/veilid/app_message.rs::handle_gossip_envelope`
    /// (verifies signature → decrements TTL → forwards even on
    /// `Ok(None)`).
    #[test]
    fn try_decode_distinguishes_malformed_from_unknown() {
        let garbage = vec![0xff_u8; 128];
        let result = try_decode_community_envelope(&garbage);
        assert!(
            matches!(result, Err(CodecError::Deserialization(_))),
            "malformed bytes should return Err(Deserialization), not Ok(None) — \
             that's reserved for known-schema-with-unknown-discriminant. Got: {result:?}"
        );
    }
}
