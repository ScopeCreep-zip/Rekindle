//! `M`/1, the presence beat (exactly 512 B): `seq`, the [`BeatState`],
//! the voice channel the member is in, the beat number (the writer's
//! 5-minute period), the community-key-sealed `SessionExtras` with its
//! generation, and secure random padding that brings the packed beat to
//! exactly [`PRESENCE_BEAT_BYTES`], all signed by the pseudonym under
//! `rekindle-presence-beat-v2` (plan E3.4 S1, Q2).

use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::keys::MediaEncryptionKey;
use rekindle_secrets::member_record::{open_member_record_field, seal_member_record_field};
use rekindle_types::domains;
use rekindle_types::presence::{SessionExtras, SessionLocation, SessionStatus};
use rekindle_utils::random::{OsRandom, RandomSource};

use super::{beat_padding, check_len, opt_text, put_lp, sign, verify, RecordScope};
use crate::capnp_codec::{bounds, capnp_err, not_in_schema, pack, unpack_bounded};
use crate::member_record_capnp::{presence_beat, session_extras_body, BeatStatus};
use crate::CodecError;

/// The size of every encoded beat: padding brings each one to exactly this
/// many bytes, so a beat's size says nothing about its fields (plan E3.4
/// Q2), and a beat of any other size is refused.
pub const PRESENCE_BEAT_BYTES: usize = 512;
/// The bound on a channel id (the voice channel, the extras' location).
pub const MAX_CHANNEL_ID: usize = 64;
/// The bound on the extras' activity, in UTF-8 bytes.
pub const MAX_ACTIVITY: usize = 96;
/// The bound on the sealed extras.
pub const MAX_SEALED_EXTRAS: usize = 256;

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
    let reader = unpack_bounded(data, bounds::PRESENCE_BEAT_BOUND)?;
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

/// What a beat says of its member: a status, or that they left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeatState {
    Online,
    Away,
    Busy,
    Offline,
    /// The member left the community (plan E3.4: leave writes it).
    Departed,
}

impl BeatState {
    /// Whether the member is present: online, away or busy.
    #[must_use]
    pub fn is_present(self) -> bool {
        matches!(self, Self::Online | Self::Away | Self::Busy)
    }

    fn wire(self) -> BeatStatus {
        match self {
            Self::Online => BeatStatus::Online,
            Self::Away => BeatStatus::Away,
            Self::Busy => BeatStatus::Busy,
            Self::Offline => BeatStatus::Offline,
            Self::Departed => BeatStatus::Departed,
        }
    }

    fn from_wire(status: BeatStatus) -> Self {
        match status {
            BeatStatus::Online => Self::Online,
            BeatStatus::Away => Self::Away,
            BeatStatus::Busy => Self::Busy,
            BeatStatus::Offline => Self::Offline,
            BeatStatus::Departed => Self::Departed,
        }
    }
}

/// `Invisible` is written as `Offline`, as every presence wire form.
impl From<SessionStatus> for BeatState {
    fn from(status: SessionStatus) -> Self {
        match status {
            SessionStatus::Online => Self::Online,
            SessionStatus::Away => Self::Away,
            SessionStatus::Busy => Self::Busy,
            SessionStatus::Offline | SessionStatus::Invisible => Self::Offline,
        }
    }
}

/// A member's liveness beat. Its padding is drawn at each encode and is
/// not kept: it carries nothing but the size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceBeat {
    /// The member's beat count, persisted before each write and never
    /// reused (`communities.beat_seq`).
    pub seq: u64,
    pub state: BeatState,
    pub voice_channel: Option<String>,
    /// `floor(writer's clock ms / BEAT_PERIOD_MS)`: the period it was
    /// written in, from which a reader's 15-minute rule counts.
    pub beat_number: u64,
    pub extras: Option<SealedExtras>,
}

/// The beat period the beat number counts (5 min, plan E3.4 "Cadence").
pub const BEAT_PERIOD_MS: u64 = 5 * 60 * 1000;

/// The beat number of `now_ms`: the 5-minute period it falls in.
#[must_use]
pub fn beat_number_at(now_ms: u64) -> u64 {
    now_ms / BEAT_PERIOD_MS
}

impl PresenceBeat {
    fn signing_bytes(&self, at: RecordScope<'_>, padding: &[u8]) -> Vec<u8> {
        let mut out = at.signing_prefix(domains::PRESENCE_BEAT);
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.push(self.state.wire() as u8);
        put_lp(
            &mut out,
            self.voice_channel.as_deref().unwrap_or("").as_bytes(),
        );
        out.extend_from_slice(&self.beat_number.to_be_bytes());
        match &self.extras {
            None => out.push(0),
            Some(extras) => {
                out.push(1);
                out.extend_from_slice(&extras.mek_generation.to_be_bytes());
                put_lp(&mut out, &extras.sealed);
            }
        }
        put_lp(&mut out, padding);
        out
    }

    /// The signed, packed beat with `padding`, at whatever size it packs to.
    fn pack_with(&self, at: RecordScope<'_>, pseudonym: &SigningKey, padding: &[u8]) -> Vec<u8> {
        let sig = sign(pseudonym, &self.signing_bytes(at, padding));
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut b = builder.init_root::<presence_beat::Builder<'_>>();
            b.set_seq(self.seq);
            b.set_status(self.state.wire());
            if let Some(voice) = self.voice_channel.as_deref().filter(|v| !v.is_empty()) {
                b.set_voice_channel(voice);
            }
            b.set_beat_number(self.beat_number);
            if let Some(extras) = &self.extras {
                let mut e = b.reborrow().init_extras();
                e.set_mek_generation(extras.mek_generation);
                e.set_sealed(&extras.sealed);
            }
            b.set_sig(&sig);
            // Allocated last, so the padding words end the message and
            // their packed cost adds to the rest (`beat_padding`).
            b.set_padding(padding);
        }
        pack(&builder)
    }

    /// Sign and encode the beat for the record `at`, padded to exactly
    /// [`PRESENCE_BEAT_BYTES`] with bytes from the OS's secure random.
    ///
    /// The padding is wire bytes that carry no meaning, like an AEAD nonce,
    /// so it never draws from a host's injected `RandomSource` (plan C9.2:
    /// a nonce never comes from a test seam; the netsim trace records
    /// semantic events, never wire bytes). Drawing it there would also
    /// shift every later id and key a seeded run draws.
    ///
    /// The padding's packed cost is chosen from the size the beat packed to
    /// with the last padding tried; the signature over each new padding
    /// changes the packed size by a few bytes (its zero bytes pack away),
    /// so the size is re-measured until it is exact.
    ///
    /// # Errors
    /// A field is over its bound, or the beat does not fit.
    pub fn encode(
        &self,
        at: RecordScope<'_>,
        pseudonym: &SigningKey,
    ) -> Result<Vec<u8>, CodecError> {
        check_len(
            "voice channel",
            self.voice_channel.as_deref().map_or(0, str::len),
            MAX_CHANNEL_ID,
        )?;
        if let Some(extras) = &self.extras {
            check_len("sealed extras", extras.sealed.len(), MAX_SEALED_EXTRAS)?;
        }
        let mut cost = beat_padding::MIN_COST;
        for _ in 0..beat_padding::MAX_ATTEMPTS {
            let out = self.pack_with(at, pseudonym, &beat_padding::padding(cost, &OsRandom));
            if out.len() == PRESENCE_BEAT_BYTES {
                return Ok(out);
            }
            cost = (cost + PRESENCE_BEAT_BYTES)
                .checked_sub(out.len())
                .filter(|c| *c >= beat_padding::MIN_COST)
                .ok_or_else(|| {
                    CodecError::Serialization(format!(
                        "presence beat packs to {} bytes, over {PRESENCE_BEAT_BYTES}",
                        out.len()
                    ))
                })?;
        }
        Err(CodecError::Serialization(
            "presence beat: no padding reached the fixed size".into(),
        ))
    }
}

/// Decode `M`/1 and verify its signature by `pseudonym` for the record
/// `at`. A beat of any size but [`PRESENCE_BEAT_BYTES`] is refused.
///
/// # Errors
/// The size is wrong, the beat does not decode, or the signature fails.
pub fn decode_presence_beat(
    data: &[u8],
    at: RecordScope<'_>,
    pseudonym: &[u8; 32],
) -> Result<PresenceBeat, CodecError> {
    if data.len() != PRESENCE_BEAT_BYTES {
        return Err(CodecError::Deserialization(format!(
            "presence beat is {} bytes, every beat is {PRESENCE_BEAT_BYTES}",
            data.len()
        )));
    }
    let reader = unpack_bounded(data, bounds::PRESENCE_BEAT_BOUND)?;
    let b = reader
        .get_root::<presence_beat::Reader<'_>>()
        .map_err(|e| capnp_err(&e))?;
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
        state: BeatState::from_wire(b.get_status().map_err(not_in_schema)?),
        voice_channel: opt_text(b.has_voice_channel(), b.get_voice_channel())?,
        beat_number: b.get_beat_number(),
        extras,
    };
    let padding = b.get_padding().map_err(|e| capnp_err(&e))?;
    verify(
        pseudonym,
        &beat.signing_bytes(at, padding),
        b.get_sig().map_err(|e| capnp_err(&e))?,
        "presence beat",
    )?;
    Ok(beat)
}
