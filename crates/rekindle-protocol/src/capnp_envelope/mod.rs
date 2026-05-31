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

use crate::capnp_codec::{capnp_err, not_in_schema, pack, text_to_string, unpack};
use crate::dht::community::envelope::{CommunityEnvelope, SignedEnvelope};
use crate::error::ProtocolError;

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

/// Encode a `CommunityEnvelope` into packed Cap'n Proto bytes.
pub fn encode_community_envelope(env: &CommunityEnvelope) -> Result<Vec<u8>, ProtocolError> {
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
pub fn decode_community_envelope(bytes: &[u8]) -> Result<CommunityEnvelope, ProtocolError> {
    let reader = unpack(bytes)?;
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
) -> Result<Option<CommunityEnvelope>, ProtocolError> {
    match decode_community_envelope(bytes) {
        Ok(env) => Ok(Some(env)),
        Err(ProtocolError::UnknownVariant(_)) => Ok(None),
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
    pack(&builder)
}

/// Decode a `SignedEnvelope` from packed Cap'n Proto bytes.
pub fn decode_signed_envelope(bytes: &[u8]) -> Result<SignedEnvelope, ProtocolError> {
    let reader = unpack(bytes)?;
    let root = reader
        .get_root::<crate::community_envelope_capnp::signed_envelope::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    Ok(SignedEnvelope {
        community_id: text_to_string(root.get_community_id().map_err(|e| capnp_err(&e))?)?,
        sender_pseudonym: text_to_string(root.get_sender_pseudonym().map_err(|e| capnp_err(&e))?)?,
        envelope_bytes: root.get_payload().map_err(|e| capnp_err(&e))?.to_vec(),
        signature: root.get_signature().map_err(|e| capnp_err(&e))?.to_vec(),
        ttl: root.get_ttl(),
    })
}

// ── Inner CommunityEnvelope writer / reader ──────────────────────────

fn write_community_envelope(
    builder: crate::community_envelope_capnp::community_envelope::Builder<'_>,
    env: &CommunityEnvelope,
) -> Result<(), ProtocolError> {
    match env {
        CommunityEnvelope::MessageNotification {
            channel_id,
            message_id,
            author_pseudonym,
            subkey_index,
            lamport_ts,
            sequence,
            content_hash,
            timestamp,
        } => {
            let mut m = builder.init_message_notification();
            m.set_channel_id(channel_id);
            m.set_message_id(message_id);
            m.set_author_pseudonym(author_pseudonym);
            m.set_subkey_index(*subkey_index);
            m.set_lamport_ts(*lamport_ts);
            m.set_sequence(*sequence);
            m.set_content_hash(content_hash);
            m.set_timestamp(*timestamp);
        }
        CommunityEnvelope::Control(payload) => {
            let ctrl = builder.init_control();
            encode_control_payload(ctrl, payload)?;
        }
        CommunityEnvelope::PresenceUpdate {
            pseudonym_key,
            status,
            game_info,
            route_blob,
        } => {
            let mut p = builder.init_presence_update();
            p.set_pseudonym_key(pseudonym_key);
            p.set_status(status);
            p.set_has_game_info(game_info.is_some());
            if let Some(gi) = game_info {
                sub_types::write_presence_game_info(p.reborrow().init_game_info(), gi);
            }
            p.set_has_route_blob(route_blob.is_some());
            if let Some(blob) = route_blob {
                p.set_route_blob(blob);
            }
        }
        CommunityEnvelope::TypingIndicator {
            channel_id,
            pseudonym_key,
        } => {
            let mut t = builder.init_typing_indicator();
            t.set_channel_id(channel_id);
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
) -> Result<CommunityEnvelope, ProtocolError> {
    use crate::community_envelope_capnp::community_envelope::Which;
    match reader.which().map_err(not_in_schema)? {
        Which::MessageNotification(m) => {
            let m = m.map_err(|e| capnp_err(&e))?;
            Ok(CommunityEnvelope::MessageNotification {
                channel_id: text_to_string(m.get_channel_id().map_err(|e| capnp_err(&e))?)?,
                message_id: text_to_string(m.get_message_id().map_err(|e| capnp_err(&e))?)?,
                author_pseudonym: text_to_string(
                    m.get_author_pseudonym().map_err(|e| capnp_err(&e))?,
                )?,
                subkey_index: m.get_subkey_index(),
                lamport_ts: m.get_lamport_ts(),
                sequence: m.get_sequence(),
                content_hash: text_to_string(m.get_content_hash().map_err(|e| capnp_err(&e))?)?,
                timestamp: m.get_timestamp(),
            })
        }
        Which::Control(c) => {
            let c = c.map_err(|e| capnp_err(&e))?;
            Ok(CommunityEnvelope::Control(decode_control_payload(c)?))
        }
        Which::PresenceUpdate(p) => {
            let p = p.map_err(|e| capnp_err(&e))?;
            let game_info = if p.get_has_game_info() {
                Some(sub_types::read_presence_game_info(
                    p.get_game_info().map_err(|e| capnp_err(&e))?,
                )?)
            } else {
                None
            };
            let route_blob = if p.get_has_route_blob() {
                Some(p.get_route_blob().map_err(|e| capnp_err(&e))?.to_vec())
            } else {
                None
            };
            Ok(CommunityEnvelope::PresenceUpdate {
                pseudonym_key: text_to_string(p.get_pseudonym_key().map_err(|e| capnp_err(&e))?)?,
                status: text_to_string(p.get_status().map_err(|e| capnp_err(&e))?)?,
                game_info,
                route_blob,
            })
        }
        Which::TypingIndicator(t) => {
            let t = t.map_err(|e| capnp_err(&e))?;
            Ok(CommunityEnvelope::TypingIndicator {
                channel_id: text_to_string(t.get_channel_id().map_err(|e| capnp_err(&e))?)?,
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
mod tests {
    use super::*;
    use crate::dht::community::envelope::PresenceGameInfo;

    #[test]
    fn signed_envelope_roundtrip() {
        let signed = SignedEnvelope {
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
    }

    #[test]
    fn typing_indicator_roundtrip() {
        let env = CommunityEnvelope::TypingIndicator {
            channel_id: "ch_01".into(),
            pseudonym_key: "abcd".into(),
        };
        let bytes = encode_community_envelope(&env).unwrap();
        let back = decode_community_envelope(&bytes).unwrap();
        match back {
            CommunityEnvelope::TypingIndicator {
                channel_id,
                pseudonym_key,
            } => {
                assert_eq!(channel_id, "ch_01");
                assert_eq!(pseudonym_key, "abcd");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn message_notification_roundtrip() {
        let env = CommunityEnvelope::MessageNotification {
            channel_id: "ch_01".into(),
            message_id: "msg_abc".into(),
            author_pseudonym: "pseudo_123".into(),
            subkey_index: 7,
            lamport_ts: 42,
            sequence: 3,
            content_hash: "abc123".into(),
            timestamp: 1_700_000_000,
        };
        let bytes = encode_community_envelope(&env).unwrap();
        let back = decode_community_envelope(&bytes).unwrap();
        match back {
            CommunityEnvelope::MessageNotification {
                channel_id,
                message_id,
                author_pseudonym,
                subkey_index,
                lamport_ts,
                sequence,
                content_hash,
                timestamp,
            } => {
                assert_eq!(channel_id, "ch_01");
                assert_eq!(message_id, "msg_abc");
                assert_eq!(author_pseudonym, "pseudo_123");
                assert_eq!(subkey_index, 7);
                assert_eq!(lamport_ts, 42);
                assert_eq!(sequence, 3);
                assert_eq!(content_hash, "abc123");
                assert_eq!(timestamp, 1_700_000_000);
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
    fn presence_update_with_game_info_roundtrip() {
        let env = CommunityEnvelope::PresenceUpdate {
            pseudonym_key: "abcd".into(),
            status: "online".into(),
            game_info: Some(PresenceGameInfo {
                game_name: "Halo".into(),
                game_id: Some(42),
                elapsed_seconds: Some(3600),
                server_address: Some("10.0.0.1:27015".into()),
            }),
            route_blob: Some(vec![9, 8, 7]),
        };
        let bytes = encode_community_envelope(&env).unwrap();
        let back = decode_community_envelope(&bytes).unwrap();
        match back {
            CommunityEnvelope::PresenceUpdate {
                pseudonym_key,
                status,
                game_info,
                route_blob,
            } => {
                assert_eq!(pseudonym_key, "abcd");
                assert_eq!(status, "online");
                let gi = game_info.expect("game info");
                assert_eq!(gi.game_name, "Halo");
                assert_eq!(gi.game_id, Some(42));
                assert_eq!(gi.elapsed_seconds, Some(3600));
                assert_eq!(gi.server_address.as_deref(), Some("10.0.0.1:27015"));
                assert_eq!(route_blob, Some(vec![9, 8, 7]));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn try_decode_returns_ok_for_known_variant() {
        let env = CommunityEnvelope::TypingIndicator {
            channel_id: "ch".into(),
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
            matches!(result, Err(ProtocolError::Deserialization(_))),
            "malformed bytes should return Err(Deserialization), not Ok(None) — \
             that's reserved for known-schema-with-unknown-discriminant. Got: {result:?}"
        );
    }
}
