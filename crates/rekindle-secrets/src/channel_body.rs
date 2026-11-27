//! The channel-message body codec every host shares (architecture §8,
//! layer 4): AES-256-GCM under the scope key with the channel body AAD
//! (plan E3.3), `domains::CHANNEL_BODY_AAD ‖ anchor ‖ channel_id ‖ author
//! ‖ stream_seq_le64` ([`ChannelAad`]).
//!
//! There is no decrypt without AAD: a body is bound to its community,
//! channel, author and the author's stream position, on every track.

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
    /// The community anchor.
    pub anchor: &'a str,
    /// The channel, or the thread for a thread's entry.
    pub channel_id: &'a str,
    /// The author's pseudonym.
    pub author: &'a [u8; 32],
    /// The entry's per-(author, tag) stream seq, frozen at creation.
    pub stream_seq: u64,
}

impl BodyPosition<'_> {
    fn aad(&self) -> Vec<u8> {
        ChannelAad {
            anchor: self.anchor,
            channel_id: self.channel_id,
            author: self.author,
            stream_seq: self.stream_seq,
        }
        .to_bytes()
    }
}

/// Encrypt a channel body under `key` (32 bytes) at `at`.
pub fn encrypt_channel_body(
    key: &[u8; 32],
    at: BodyPosition<'_>,
    body: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    MediaEncryptionKey::from_bytes(*key, 0).encrypt_with_aad(body, &at.aad())
}

/// Decrypt a body written by [`encrypt_channel_body`] at the same `at`.
pub fn decrypt_channel_body(
    key: &[u8; 32],
    at: BodyPosition<'_>,
    ciphertext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    MediaEncryptionKey::from_bytes(*key, 0).decrypt_with_aad(ciphertext, &at.aad())
}

#[cfg(test)]
#[path = "channel_body_tests.rs"]
mod tests;
