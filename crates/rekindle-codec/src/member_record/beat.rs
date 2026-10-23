//! `M`/1, the presence beat (≤ 512 B): `seq`, status, the voice channel
//! the member is in, when the beat expires, and the community-key-sealed
//! `SessionExtras` with its generation, signed by the pseudonym under
//! `rekindle-presence-beat-v1`. E3.4 builds the liveness engine on it.

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::keys::MediaEncryptionKey;
use rekindle_secrets::member_record::{open_member_record_field, seal_member_record_field};
use rekindle_types::domains;
use rekindle_types::presence::{SessionExtras, SessionLocation, SessionStatus};
use rekindle_utils::random::RandomSource;

use super::{check_len, check_size, opt_text, put_lp, sign, verify, RecordScope};
use crate::capnp_codec::{capnp_err, not_in_schema, pack, unpack_bounded};
use crate::member_record_capnp::{presence_beat, session_extras_body, BeatStatus};
use crate::CodecError;

/// The bound on an encoded beat, proven at the maximum field sizes.
pub const PRESENCE_BEAT_MAX_BYTES: usize = 512;
/// The bound on a channel id (the voice channel, the extras' location).
pub const MAX_CHANNEL_ID: usize = 64;
/// The bound on the extras' activity, in UTF-8 bytes.
pub const MAX_ACTIVITY: usize = 96;
/// The bound on the sealed extras.
pub const MAX_SEALED_EXTRAS: usize = 256;

const MAX_WORDS: usize = 256;

/// The `SessionExtras` sealed under the community text key of
/// `mek_generation`, bound to the record it is written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedExtras {
    pub mek_generation: u64,
    pub sealed: Vec<u8>,
}

impl SealedExtras {
    /// Seal `extras` under `key`, the current `KeyScope::Community` key.
    pub fn seal(
        extras: &SessionExtras,
        key: &MediaEncryptionKey,
        at: RecordScope<'_>,
        rng: &dyn RandomSource,
    ) -> Result<Self, CodecError> {
        let plaintext = encode_extras(extras)?;
        let sealed = seal_member_record_field(key, at.aad(), &plaintext, rng)
            .map_err(|e| CodecError::Serialization(e.to_string()))?;
        check_len("sealed extras", sealed.len(), MAX_SEALED_EXTRAS)?;
        Ok(Self {
            mek_generation: key.generation(),
            sealed,
        })
    }

    /// Open the extras with `key`, which must be of their generation.
    pub fn open(
        &self,
        key: &MediaEncryptionKey,
        at: RecordScope<'_>,
    ) -> Result<SessionExtras, CodecError> {
        if key.generation() != self.mek_generation {
            return Err(CodecError::Verification(format!(
                "extras sealed at generation {}, key is {}",
                self.mek_generation,
                key.generation()
            )));
        }
        let plaintext = open_member_record_field(key, at.aad(), &self.sealed)
            .map_err(|e| CodecError::Verification(e.to_string()))?;
        decode_extras(&plaintext)
    }
}

fn encode_extras(extras: &SessionExtras) -> Result<Vec<u8>, CodecError> {
    check_len(
        "activity",
        extras.activity.as_ref().map_or(0, String::len),
        MAX_ACTIVITY,
    )?;
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut b = builder.init_root::<session_extras_body::Builder<'_>>();
        match &extras.location {
            None => b.reborrow().init_location().set_none(()),
            Some(SessionLocation::Text { channel_id }) => {
                check_len("location channel", channel_id.len(), MAX_CHANNEL_ID)?;
                b.reborrow().init_location().set_text(channel_id.as_str());
            }
            Some(SessionLocation::Voice { channel_id }) => {
                check_len("location channel", channel_id.len(), MAX_CHANNEL_ID)?;
                b.reborrow().init_location().set_voice(channel_id.as_str());
            }
        }
        if let Some(activity) = &extras.activity {
            b.set_activity(activity.as_str());
        }
    }
    Ok(pack(&builder))
}

fn decode_extras(data: &[u8]) -> Result<SessionExtras, CodecError> {
    let reader = unpack_bounded(data, MAX_WORDS)?;
    let b = reader
        .get_root::<session_extras_body::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    use session_extras_body::location::Which;
    let text = |t: Result<capnp::text::Reader<'_>, capnp::Error>| {
        opt_text(true, t)?.ok_or_else(|| CodecError::Deserialization("empty location".into()))
    };
    let location = match b.get_location().which().map_err(not_in_schema)? {
        Which::None(()) => None,
        Which::Text(t) => Some(SessionLocation::Text {
            channel_id: text(t)?,
        }),
        Which::Voice(t) => Some(SessionLocation::Voice {
            channel_id: text(t)?,
        }),
    };
    Ok(SessionExtras {
        location,
        activity: opt_text(b.has_activity(), b.get_activity())?,
    })
}

/// A member's liveness beat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceBeat {
    pub seq: u64,
    /// `Invisible` is written as `Offline`, as every presence wire form.
    pub status: SessionStatus,
    pub voice_channel: Option<String>,
    /// When the beat lapses, on the writer's clock (ms).
    pub expires_at: u64,
    pub extras: Option<SealedExtras>,
}

fn wire_status(status: SessionStatus) -> BeatStatus {
    match status {
        SessionStatus::Online => BeatStatus::Online,
        SessionStatus::Away => BeatStatus::Away,
        SessionStatus::Busy => BeatStatus::Busy,
        SessionStatus::Offline | SessionStatus::Invisible => BeatStatus::Offline,
    }
}

impl PresenceBeat {
    fn signing_bytes(&self, at: RecordScope<'_>, status: BeatStatus) -> Vec<u8> {
        let mut out = at.signing_prefix(domains::PRESENCE_BEAT);
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.push(status as u8);
        put_lp(
            &mut out,
            self.voice_channel.as_deref().unwrap_or("").as_bytes(),
        );
        out.extend_from_slice(&self.expires_at.to_be_bytes());
        match &self.extras {
            None => out.push(0),
            Some(extras) => {
                out.push(1);
                out.extend_from_slice(&extras.mek_generation.to_be_bytes());
                put_lp(&mut out, &extras.sealed);
            }
        }
        out
    }

    /// Sign and encode the beat for the record `at`.
    pub fn encode(
        &self,
        at: RecordScope<'_>,
        pseudonym: &SigningKey,
    ) -> Result<Vec<u8>, CodecError> {
        let voice = self.voice_channel.as_deref().unwrap_or("");
        check_len("voice channel", voice.len(), MAX_CHANNEL_ID)?;
        if let Some(extras) = &self.extras {
            check_len("sealed extras", extras.sealed.len(), MAX_SEALED_EXTRAS)?;
        }
        let status = wire_status(self.status);
        let sig = sign(pseudonym, &self.signing_bytes(at, status));
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut b = builder.init_root::<presence_beat::Builder<'_>>();
            b.set_seq(self.seq);
            b.set_status(status);
            if !voice.is_empty() {
                b.set_voice_channel(voice);
            }
            b.set_expires_at(self.expires_at);
            if let Some(extras) = &self.extras {
                let mut e = b.reborrow().init_extras();
                e.set_mek_generation(extras.mek_generation);
                e.set_sealed(&extras.sealed);
            }
            b.set_sig(&sig);
        }
        let out = pack(&builder);
        check_size("presence beat", &out, PRESENCE_BEAT_MAX_BYTES)?;
        Ok(out)
    }
}

/// Decode `M`/1 and verify its signature by `pseudonym` for the record `at`.
pub fn decode_presence_beat(
    data: &[u8],
    at: RecordScope<'_>,
    pseudonym: &[u8; 32],
) -> Result<PresenceBeat, CodecError> {
    check_size("presence beat", data, PRESENCE_BEAT_MAX_BYTES)?;
    let reader = unpack_bounded(data, MAX_WORDS)?;
    let b = reader
        .get_root::<presence_beat::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
    let status = b.get_status().map_err(not_in_schema)?;
    let extras = if b.has_extras() {
        let e = b.get_extras().map_err(|e| capnp_err(&e))?;
        Some(SealedExtras {
            mek_generation: e.get_mek_generation(),
            sealed: e.get_sealed().map_err(|e| capnp_err(&e))?.to_vec(),
        })
    } else {
        None
    };
    let beat = PresenceBeat {
        seq: b.get_seq(),
        status: match status {
            BeatStatus::Online => SessionStatus::Online,
            BeatStatus::Away => SessionStatus::Away,
            BeatStatus::Busy => SessionStatus::Busy,
            BeatStatus::Offline => SessionStatus::Offline,
        },
        voice_channel: opt_text(b.has_voice_channel(), b.get_voice_channel())?,
        expires_at: b.get_expires_at(),
        extras,
    };
    verify(
        pseudonym,
        &beat.signing_bytes(at, status),
        b.get_sig().map_err(|e| capnp_err(&e))?,
        "presence beat",
    )?;
    Ok(beat)
}
