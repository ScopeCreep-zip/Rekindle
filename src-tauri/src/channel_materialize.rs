//! Opening a channel message body read from the channel record (plan D6).
//! The history read and its merge moved to the Host at E3.1 E2c
//! (`rekindle_channel::history`); the desktop's presence and notification
//! paths still open bodies here until they are ported.

use crate::state::SharedState;
use crate::state_helpers;

pub(crate) struct DecryptedMessageBody {
    pub body: String,
    pub decryption_failed: bool,
}

/// Open a channel message body: exactly the generation it names of the
/// channel's text key (plan D6), bound to the record, subkey and Lamport
/// position it was read from (architecture §8). There is no other key and
/// no decrypt without the position — a body that does not open there is
/// `decryption_failed`, and a generation we do not hold stays unreadable
/// until the key arrives.
pub(crate) fn decrypt_channel_record_message(
    state: &SharedState,
    community_id: &str,
    channel_id: &str,
    mek_generation: u64,
    ciphertext: &[u8],
    at: rekindle_secrets::channel_body::BodyPosition<'_>,
) -> DecryptedMessageBody {
    let opened = rekindle_types::id::ChannelId::from_hex(channel_id).and_then(|channel| {
        let keys = state_helpers::key_provider(state);
        let scope = keys.scope_for_text(community_id, channel);
        let key = keys.key(
            community_id,
            scope,
            rekindle_types::channel_keys::KeyEpoch(mek_generation),
        )?;
        rekindle_secrets::channel_body::decrypt_channel_body(&key, at, ciphertext).ok()
    });
    match opened {
        Some(bytes) => DecryptedMessageBody {
            body: String::from_utf8_lossy(&bytes).into_owned(),
            decryption_failed: false,
        },
        None => DecryptedMessageBody {
            body: String::new(),
            decryption_failed: true,
        },
    }
}
