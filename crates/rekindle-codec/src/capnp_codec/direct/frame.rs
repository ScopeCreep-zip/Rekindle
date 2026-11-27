//! `DmFrame` and its plaintexts (`DmContent`, `SealedContent`).

use rekindle_types::dht_layout::MAX_APP_MESSAGE_LEN;

use super::{data, decode_root, encode_root, fixed, Carrier, DmTypeId};
use crate::capnp_codec::{capnp_err, not_in_schema, CodecError};
use crate::direct_capnp::{
    dm_content, dm_frame, pre_key_init_body, ratchet_body, ratchet_header, sealed_content,
    ContentHint as WireHint,
};

/// One 1:1 frame: the kind, the sender's identity, and the sealed body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmFrame {
    pub type_id: DmTypeId,
    /// The sender's Ed25519 identity (identity == address).
    pub sender: [u8; 32],
    pub body: FrameBody,
}

/// How a frame is sealed; the one its typeId's [`Carrier`] names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameBody {
    Ratchet(RatchetBody),
    PreKeyInit(PreKeyInitBody),
    Sealed(SealedBody),
}

/// The Double Ratchet header (rev. 1 §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatchetHeader {
    pub dh: [u8; 32],
    pub pn: u32,
    pub n: u32,
}

/// One ratchet message: its header and the AEAD ciphertext of a
/// [`DmContent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RatchetBody {
    pub header: RatchetHeader,
    pub ciphertext: Vec<u8>,
}

/// The responder ML-KEM prekey a PreKeyInit's KEM ciphertext is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PqPrekey {
    OneTime(u32),
    LastResort(u32),
}

/// The initiator's first message (PQXDH rev. 3 §3.3); `IK_A` is the
/// frame's sender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreKeyInitBody {
    /// `EK_A`'s public half: the session's base key.
    pub base_key: [u8; 32],
    pub signed_prekey_id: u32,
    /// The responder's one-time curve prekey, when DH4 ran.
    pub one_time_prekey_id: Option<u32>,
    pub pq_prekey: PqPrekey,
    /// `CT`.
    pub kem_ciphertext: Vec<u8>,
    /// The kind of the encrypted first payload.
    pub first_type_id: DmTypeId,
    /// The first [`DmContent`], encrypted under the new session.
    pub message: RatchetBody,
}

/// A pre-session frame: HPKE Base to the recipient identity, signed by
/// the sender's identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedBody {
    /// The HPKE encapsulated key.
    pub enc: [u8; 32],
    /// The HPKE ciphertext of a [`SealedContent`].
    pub ciphertext: Vec<u8>,
    /// Ed25519 over `DM_PRESESSION_SIG ‖ recipient ‖ typeId ‖ enc ‖ ct`.
    pub signature: [u8; 64],
}

/// The D24 content hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentHint {
    Default,
    Resendable,
    Implicit,
}

/// The plaintext of every ratchet and PreKeyInit message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmContent {
    pub content_hint: ContentHint,
    /// The sender's clock, milliseconds.
    pub sent_at_ms: u64,
    /// The kind's body.
    pub payload: Vec<u8>,
    /// The entry's seq on the sender's outbox stream to us (plan E2.5,
    /// D22), which our `DmAck` names; 0 for a frame on no stream.
    pub stream_seq: u64,
    /// The entry's seq on the sender's dm chain to us, sealed into its hint
    /// (live A/B finding 71 refined): the receiver applies D23's rule to it;
    /// 0 for a frame on no chain (a `StreamRecord`, a group entry).
    pub chain_seq: u64,
}

/// The plaintext of a sealed frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedContent {
    /// Random; the replay guard keeps `(sender, nonce)`.
    pub nonce: [u8; 16],
    /// The sender's clock, milliseconds.
    pub sent_at_ms: u64,
    /// The kind's body.
    pub payload: Vec<u8>,
}

impl DmFrame {
    /// Packed Cap'n Proto bytes.
        pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        encode_root::<dm_frame::Owned>(|mut root| {
            root.set_type_id(self.type_id.as_u16());
            root.set_sender(&self.sender);
            let body = root.init_body();
            match &self.body {
                FrameBody::Ratchet(r) => write_ratchet(body.init_ratchet(), r),
                FrameBody::PreKeyInit(p) => write_pre_key_init(body.init_pre_key_init(), p),
                FrameBody::Sealed(s) => {
                    let mut b = body.init_sealed();
                    b.set_enc(&s.enc);
                    b.set_ciphertext(&s.ciphertext);
                    b.set_signature(&s.signature);
                }
            }
            Ok(())
        })
    }

    /// Decode and shape-check packed bytes (the datagram after its `'D'`
    /// tag): bytes that with the tag would be over Veilid's message cap are
    /// refused before they are unpacked; an unallocated typeId is
    /// `UnknownVariant`, a body other than the typeId's carrier names is
    /// `Deserialization`.
    ///
    /// # Errors
    /// The bytes are not a well-formed `DmFrame`.
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() >= MAX_APP_MESSAGE_LEN {
            return Err(CodecError::Deserialization(format!(
                "a {}-byte frame is over the {MAX_APP_MESSAGE_LEN}-byte message cap",
                // A slice is at most `isize::MAX` bytes: never saturates.
                bytes.len().saturating_add(1)
            )));
        }
        let frame = decode_root::<dm_frame::Owned, _>(bytes, |root| {
            let type_id = kind(root.get_type_id())?;
            let body = match root.get_body().which().map_err(not_in_schema)? {
                dm_frame::body::Ratchet(r) => {
                    FrameBody::Ratchet(read_ratchet(r.map_err(|e| capnp_err(&e))?)?)
                }
                dm_frame::body::PreKeyInit(p) => {
                    FrameBody::PreKeyInit(read_pre_key_init(p.map_err(|e| capnp_err(&e))?)?)
                }
                dm_frame::body::Sealed(s) => {
                    let s = s.map_err(|e| capnp_err(&e))?;
                    FrameBody::Sealed(SealedBody {
                        enc: fixed(s.get_enc(), "sealed enc")?,
                        ciphertext: data(s.get_ciphertext())?,
                        signature: fixed(s.get_signature(), "sealed signature")?,
                    })
                }
            };
            Ok(Self {
                type_id,
                sender: fixed(root.get_sender(), "frame sender")?,
                body,
            })
        })?;
        frame.check_shape()?;
        Ok(frame)
    }

    /// The body is the one the typeId's carrier names, and a PreKeyInit's
    /// first payload is a kind it may carry.
    fn check_shape(&self) -> Result<(), CodecError> {
        let fits = match (&self.body, self.type_id.carrier()) {
            // A group entry rides a member's pairwise ratchet only as its
            // hint (or as typing, which is never on a chain).
            (FrameBody::Ratchet(_), Carrier::Ratchet | Carrier::GroupChain)
            | (FrameBody::Sealed(_), Carrier::Sealed) => true,
            (FrameBody::PreKeyInit(init), Carrier::PreKeyInit) => {
                init.first_type_id.is_first_payload()
            }
            _ => false,
        };
        if fits {
            Ok(())
        } else {
            Err(CodecError::Deserialization(format!(
                "{:?} frame with the wrong body",
                self.type_id
            )))
        }
    }
}

fn kind(value: u16) -> Result<DmTypeId, CodecError> {
    DmTypeId::from_u16(value)
        .ok_or_else(|| CodecError::UnknownVariant(format!("DmFrame typeId {value:#06x}")))
}

fn write_ratchet(mut b: ratchet_body::Builder<'_>, body: &RatchetBody) {
    write_header(b.reborrow().init_header(), body.header);
    b.set_ciphertext(&body.ciphertext);
}

fn write_header(mut b: ratchet_header::Builder<'_>, header: RatchetHeader) {
    b.set_dh(&header.dh);
    b.set_pn(header.pn);
    b.set_n(header.n);
}

fn read_ratchet(r: ratchet_body::Reader<'_>) -> Result<RatchetBody, CodecError> {
    let h = r.get_header().map_err(|e| capnp_err(&e))?;
    Ok(RatchetBody {
        header: RatchetHeader {
            dh: fixed(h.get_dh(), "ratchet header dh")?,
            pn: h.get_pn(),
            n: h.get_n(),
        },
        ciphertext: data(r.get_ciphertext())?,
    })
}

fn write_pre_key_init(mut b: pre_key_init_body::Builder<'_>, init: &PreKeyInitBody) {
    b.set_base_key(&init.base_key);
    b.set_signed_prekey_id(init.signed_prekey_id);
    match init.one_time_prekey_id {
        Some(id) => b.reborrow().init_one_time_prekey().set_id(id),
        None => b.reborrow().init_one_time_prekey().set_none(()),
    }
    match init.pq_prekey {
        PqPrekey::OneTime(id) => b.reborrow().init_pq_prekey().set_one_time(id),
        PqPrekey::LastResort(id) => b.reborrow().init_pq_prekey().set_last_resort(id),
    }
    b.set_kem_ciphertext(&init.kem_ciphertext);
    b.set_first_type_id(init.first_type_id.as_u16());
    write_ratchet(b.init_message(), &init.message);
}

fn read_pre_key_init(r: pre_key_init_body::Reader<'_>) -> Result<PreKeyInitBody, CodecError> {
    use pre_key_init_body::{one_time_prekey, pq_prekey};
    let one_time_prekey_id = match r.get_one_time_prekey().which().map_err(not_in_schema)? {
        one_time_prekey::None(()) => None,
        one_time_prekey::Id(id) => Some(id),
    };
    let pq_prekey = match r.get_pq_prekey().which().map_err(not_in_schema)? {
        pq_prekey::OneTime(id) => PqPrekey::OneTime(id),
        pq_prekey::LastResort(id) => PqPrekey::LastResort(id),
    };
    Ok(PreKeyInitBody {
        base_key: fixed(r.get_base_key(), "PreKeyInit base key")?,
        signed_prekey_id: r.get_signed_prekey_id(),
        one_time_prekey_id,
        pq_prekey,
        kem_ciphertext: data(r.get_kem_ciphertext())?,
        first_type_id: kind(r.get_first_type_id())?,
        message: read_ratchet(r.get_message().map_err(|e| capnp_err(&e))?)?,
    })
}

impl DmContent {
    /// Packed Cap'n Proto bytes.
        pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        encode_root::<dm_content::Owned>(|mut root| {
            root.set_content_hint(match self.content_hint {
                ContentHint::Default => WireHint::Default,
                ContentHint::Resendable => WireHint::Resendable,
                ContentHint::Implicit => WireHint::Implicit,
            });
            root.set_sent_at_ms(self.sent_at_ms);
            root.set_payload(&self.payload);
            root.set_stream_seq(self.stream_seq);
            root.set_chain_seq(self.chain_seq);
            Ok(())
        })
    }

    /// Decode packed bytes.
    ///
    /// # Errors
    /// The bytes are not a `DmContent`.
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        decode_root::<dm_content::Owned, _>(bytes, |root| {
            Ok(Self {
                content_hint: match root.get_content_hint().map_err(not_in_schema)? {
                    WireHint::Default => ContentHint::Default,
                    WireHint::Resendable => ContentHint::Resendable,
                    WireHint::Implicit => ContentHint::Implicit,
                },
                sent_at_ms: root.get_sent_at_ms(),
                payload: data(root.get_payload())?,
                stream_seq: root.get_stream_seq(),
                chain_seq: root.get_chain_seq(),
            })
        })
    }
}

impl SealedContent {
    /// Packed Cap'n Proto bytes.
        pub fn encode(&self) -> Result<Vec<u8>, CodecError> {
        encode_root::<sealed_content::Owned>(|mut root| {
            root.set_nonce(&self.nonce);
            root.set_sent_at_ms(self.sent_at_ms);
            root.set_payload(&self.payload);
            Ok(())
        })
    }

    /// Decode packed bytes.
    ///
    /// # Errors
    /// The bytes are not a `SealedContent`.
    pub fn decode(bytes: &[u8]) -> Result<Self, CodecError> {
        decode_root::<sealed_content::Owned, _>(bytes, |root| {
            Ok(Self {
                nonce: fixed(root.get_nonce(), "sealed nonce")?,
                sent_at_ms: root.get_sent_at_ms(),
                payload: data(root.get_payload())?,
            })
        })
    }
}

#[cfg(test)]
#[path = "frame_tests.rs"]
pub(super) mod tests;
