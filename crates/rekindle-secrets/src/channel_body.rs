//! The channel-message body codec every host shares (architecture §8,
//! layer 4): AES-256-GCM under the scope key with the canonical channel
//! AAD, `channel_record_key ‖ subkey_index_le32 ‖ lamport_ts_le64`.
//!
//! There is no decrypt without AAD: a body is bound to the channel record,
//! slot and Lamport position it was written at, on every track.

use crate::keys::{ChannelAad, MediaEncryptionKey};
use hkdf::Hkdf;
use rekindle_types::domains;
use rekindle_types::error::CryptoError;
use sha2::Sha256;

/// A channel tag's length: the first 16 bytes of the HKDF output.
pub const CHANNEL_TAG_LEN: usize = 16;

/// The tag an outbox entry of `stream_id` (a channel id, or a thread id for
/// a thread) carries in clear under the text key of its generation (plan
/// E3.3): `HKDF(key, domains::CHANNEL_TAG ‖ stream_id)[..16]`. Only a
/// holder of that generation can tell which entries belong to the stream.
#[must_use]
pub fn channel_tag(key: &[u8; 32], stream_id: &str) -> [u8; CHANNEL_TAG_LEN] {
    let mut info = Vec::with_capacity(domains::CHANNEL_TAG.len() + stream_id.len());
    info.extend_from_slice(domains::CHANNEL_TAG.as_bytes());
    info.extend_from_slice(stream_id.as_bytes());
    let mut tag = [0u8; CHANNEL_TAG_LEN];
    Hkdf::<Sha256>::new(None, key)
        .expand(&info, &mut tag)
        .expect("16-byte output is a valid HKDF-SHA256 length");
    tag
}

/// Where a channel body is written: the AAD it is bound to.
#[derive(Debug, Clone, Copy)]
pub struct BodyPosition<'a> {
    pub channel_record_key: &'a str,
    pub subkey_index: u32,
    pub lamport_ts: u64,
}

impl BodyPosition<'_> {
    fn aad(&self) -> ChannelAad<'_> {
        ChannelAad {
            channel_record_key: self.channel_record_key.as_bytes(),
            subkey_index: self.subkey_index,
            lamport_ts: self.lamport_ts,
        }
    }
}

/// Encrypt a channel body under `key` (32 bytes) at `at`.
pub fn encrypt_channel_body(
    key: &[u8; 32],
    at: BodyPosition<'_>,
    body: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    MediaEncryptionKey::from_bytes(*key, 0).encrypt_with_aad(body, at.aad())
}

/// Decrypt a body written by [`encrypt_channel_body`] at the same `at`.
pub fn decrypt_channel_body(
    key: &[u8; 32],
    at: BodyPosition<'_>,
    ciphertext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    MediaEncryptionKey::from_bytes(*key, 0).decrypt_with_aad(ciphertext, at.aad())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: BodyPosition<'static> = BodyPosition {
        channel_record_key: "VLD0:record",
        subkey_index: 3,
        lamport_ts: 42,
    };

    #[test]
    fn round_trip() {
        let ct = encrypt_channel_body(&[1u8; 32], AT, b"hello").unwrap();
        assert_eq!(decrypt_channel_body(&[1u8; 32], AT, &ct).unwrap(), b"hello");
    }

    /// Moving a body to another record, slot or Lamport position fails.
    #[test]
    fn bound_to_its_position() {
        let ct = encrypt_channel_body(&[1u8; 32], AT, b"hello").unwrap();
        for moved in [
            BodyPosition {
                channel_record_key: "VLD0:other",
                ..AT
            },
            BodyPosition {
                subkey_index: 4,
                ..AT
            },
            BodyPosition {
                lamport_ts: 43,
                ..AT
            },
        ] {
            assert!(decrypt_channel_body(&[1u8; 32], moved, &ct).is_err());
        }
    }

    /// A tag is fixed by the key and the stream: another generation's key
    /// or another channel gives another tag.
    #[test]
    fn a_tag_is_bound_to_its_key_and_stream() {
        let tag = channel_tag(&[1u8; 32], "chan-a");
        assert_eq!(tag, channel_tag(&[1u8; 32], "chan-a"));
        assert_ne!(tag, channel_tag(&[2u8; 32], "chan-a"));
        assert_ne!(tag, channel_tag(&[1u8; 32], "chan-b"));
    }

    /// A body encrypted without AAD (the old path) does not decrypt.
    #[test]
    fn no_aadless_bodies() {
        let legacy = MediaEncryptionKey::from_bytes([1u8; 32], 0)
            .encrypt(b"hello")
            .unwrap();
        assert!(decrypt_channel_body(&[1u8; 32], AT, &legacy).is_err());
    }
}
