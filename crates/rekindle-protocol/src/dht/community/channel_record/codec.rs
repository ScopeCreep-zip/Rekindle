//! Entry encode/decode with author-signature verification.

use super::types::{ChannelMessage, ChannelRecordEntry, ChannelSubkeyPayload};
use crate::error::ProtocolError;

/// Decode raw subkey bytes into the entry vec, verifying the author's
/// pseudonym signature AND that every entry's per-record sender field
/// matches the wrapper-level author. Returns `Err` for any signature
/// failure, length mismatch, or per-entry sender forgery so callers
/// don't quietly treat forged entries as authentic.
///
/// Architecture §26 W26 — three entry variants carry their own sender
/// field (`ChannelMessage.sender_pseudonym`,
/// `ChannelForward.sender_pseudonym`,
/// `ChannelAttachmentCached.author_pseudonym`). Without this binding the
/// wrapper signature would only prove "signed by author X"; X could
/// then write entries claiming `sender_pseudonym = victim` and the
/// receiver would attribute them to victim. We reject the entire
/// payload on any mismatch — a single forged entry taints the rest
/// because they all flowed through the same writer who tried to lie.
pub fn decode_channel_entries(data: &[u8]) -> Result<Vec<ChannelRecordEntry>, ProtocolError> {
    decode_payload(data).map(|(_, entries)| entries)
}

/// The entries of a page `author` wrote, verified as
/// [`decode_channel_entries`]; `None` for anyone else's page or a page that
/// fails verification. This is what an append inherits and what its
/// compare-and-swap merges (§26 W26): a page another key wrote into our
/// slot is neither re-signed as ours nor kept (plan C7.13).
pub(super) fn decode_own_page(
    data: &[u8],
    author: &rekindle_types::id::PseudonymKey,
) -> Option<Vec<ChannelRecordEntry>> {
    match decode_payload(data) {
        Ok((page_author, entries)) if page_author == *author => Some(entries),
        _ => None,
    }
}

fn decode_payload(
    data: &[u8],
) -> Result<(rekindle_types::id::PseudonymKey, Vec<ChannelRecordEntry>), ProtocolError> {
    let payload: ChannelSubkeyPayload = serde_json::from_slice(data)
        .map_err(|e| ProtocolError::Deserialization(format!("channel subkey: {e}")))?;
    let sig_arr: [u8; 64] = payload
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| ProtocolError::Verification("channel subkey signature length".into()))?;
    rekindle_secrets::derive::verify_pseudonym_signature(
        &payload.author_pseudonym.0,
        &payload.signing_bytes(),
        &sig_arr,
    )
    .map_err(|e| ProtocolError::Verification(format!("channel subkey signature: {e}")))?;
    let author_hex = hex::encode(payload.author_pseudonym.0);
    for entry in &payload.entries {
        match entry {
            ChannelRecordEntry::Message(msg) => {
                if !msg.sender_pseudonym.eq_ignore_ascii_case(&author_hex) {
                    return Err(ProtocolError::Verification(
                        "channel message sender_pseudonym does not match subkey author".into(),
                    ));
                }
            }
            ChannelRecordEntry::Forward(fwd) => {
                if !fwd.sender_pseudonym.eq_ignore_ascii_case(&author_hex) {
                    return Err(ProtocolError::Verification(
                        "channel forward sender_pseudonym does not match subkey author".into(),
                    ));
                }
            }
            ChannelRecordEntry::AttachmentCached(att) => {
                if !att.author_pseudonym.eq_ignore_ascii_case(&author_hex) {
                    return Err(ProtocolError::Verification(
                        "channel attachment-cached author_pseudonym does not match subkey author"
                            .into(),
                    ));
                }
            }
            ChannelRecordEntry::Reaction(_)
            | ChannelRecordEntry::HandRaise(_)
            | ChannelRecordEntry::PollCreate(_)
            | ChannelRecordEntry::PollVote(_)
            | ChannelRecordEntry::PollClose(_) => {
                // No sender field; attribution is by subkey ownership
                // (the wrapper-level author), which is already verified.
            }
        }
    }
    Ok((payload.author_pseudonym, payload.entries))
}

pub(super) fn encode_page_entries(
    author_pseudonym: rekindle_types::id::PseudonymKey,
    pseudonym_signing_key: &ed25519_dalek::SigningKey,
    entries: Vec<ChannelRecordEntry>,
) -> Result<Vec<u8>, ProtocolError> {
    let mut payload = ChannelSubkeyPayload {
        author_pseudonym,
        entries,
        signature: Vec::new(),
    };
    let sig = rekindle_secrets::derive::sign_with_pseudonym(
        pseudonym_signing_key,
        &payload.signing_bytes(),
    );
    payload.signature = sig.to_vec();
    serde_json::to_vec(&payload)
        .map_err(|e| ProtocolError::Serialization(format!("channel subkey: {e}")))
}

pub(super) fn message_from_entry(entry: &ChannelRecordEntry) -> Option<&ChannelMessage> {
    match entry {
        ChannelRecordEntry::Message(message) => Some(message),
        ChannelRecordEntry::Forward(_)
        | ChannelRecordEntry::AttachmentCached(_)
        | ChannelRecordEntry::Reaction(_)
        | ChannelRecordEntry::PollCreate(_)
        | ChannelRecordEntry::PollVote(_)
        | ChannelRecordEntry::PollClose(_)
        | ChannelRecordEntry::HandRaise(_) => None,
    }
}
