//! The bare media-key call dialect shared by the desktop and transport.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use rekindle_codec::capnp_envelope::encode_community_envelope;
use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_gossip::rate_limit::TokenBucket;
use rekindle_types::key_format::pseudonym_hex;

/// Burst of key requests allowed per claimed requester.
pub const MEDIA_KEY_BURST: u32 = 8;
const REFILL_PER_SECOND: u32 = 2;
const MAX_REQUESTERS: usize = 1024;
const IDLE_TTL: Duration = Duration::from_secs(60);

/// Admission for control envelopes addressed to the media-key service.
/// The key's signed payload is checked by the receiving voice engine.
/// Unsigned requests additionally spend a token before any handler runs.
#[derive(Debug, Default)]
pub struct MediaKeyAdmission {
    buckets: HashMap<[u8; 32], TokenBucket>,
    next_prune: Option<Instant>,
}

impl MediaKeyAdmission {
    /// Admit a key or meter a request under its claimed pseudonym.
    pub fn admit(&mut self, payload: &ControlPayload, now: Instant) -> bool {
        let requester = match payload {
            ControlPayload::VoiceMediaKey { .. } => return true,
            ControlPayload::VoiceMediaKeyRequest { requester, .. } => requester,
            _ => return false,
        };
        let Ok(requester) = pseudonym_hex(requester) else {
            return false;
        };
        let requester = requester.to_bytes();
        if let Some(bucket) = self.buckets.get_mut(&requester) {
            return bucket.try_consume_at(1, now);
        }
        // A claimed pseudonym is cheap to invent. Bound memory and the
        // work spent scanning idle buckets even under new-name traffic.
        if self.buckets.len() >= MAX_REQUESTERS && self.next_prune.is_none_or(|due| now >= due) {
            self.buckets
                .retain(|_, bucket| now.saturating_duration_since(bucket.last_refill()) < IDLE_TTL);
            self.next_prune = Some(now + Duration::from_secs(1));
        }
        if self.buckets.len() >= MAX_REQUESTERS {
            return false;
        }
        let mut bucket = TokenBucket::new(MEDIA_KEY_BURST, REFILL_PER_SECOND, now);
        let admitted = bucket.try_consume_at(1, now);
        self.buckets.insert(requester, bucket);
        admitted
    }
}

/// Results of voice policy after the bare call passes transport admission.
pub enum MediaKeyReply {
    /// A key was opened and installed. Carries its `VoiceMediaKeyAck`.
    Installed(Box<ControlPayload>),
    /// A request passed the voice session's roster and recipient checks.
    Requested,
    /// The voice session refused the call.
    Refused,
}

impl MediaKeyReply {
    /// Encode the response in the caller's dialect. `None` means NAK.
    #[must_use]
    pub fn encode(self, call: &ControlPayload) -> Option<Vec<u8>> {
        match self {
            Self::Installed(ack) if call.acknowledged_by(&ack) => {
                match encode_community_envelope(&CommunityEnvelope::control(*ack)) {
                    Ok(bytes) => Some(bytes),
                    Err(error) => {
                        tracing::warn!(%error, "could not encode media-key acknowledgement");
                        None
                    }
                }
            }
            Self::Requested if matches!(call, ControlPayload::VoiceMediaKeyRequest { .. }) => {
                Some(Vec::new())
            }
            _ => None,
        }
    }
}
