//! Call bodies, moved unchanged from the pre-E2.3 1:1 payload and
//! now ratchet-sealed (E4.2 replaces them with `MediaSignal`), and DM
//! video until E4.3.8.

use crate::capnp_codec::direct::{data, fixed, list_len, set_text_list, text, text_list};
use crate::capnp_codec::CodecError;
use crate::direct_capnp::{
    call_accept, call_closed, call_invite, call_media_state, call_reaction, call_ringing,
    dm_video_fragment, group_call_member, group_call_offer, group_call_participant_left,
};

/// 0x0200 CallInvite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallInvite {
    pub call_id: String,
    /// 0 = audio, 1 = video.
    pub offer_kind: u8,
    pub initiator_pubkey: String,
    pub initiator_x25519_pub: Vec<u8>,
    pub expires_at_ms: u64,
    pub video_decode_codecs: Vec<String>,
}

impl CallInvite {
    pub(super) fn write(&self, mut b: call_invite::Builder<'_>) {
        b.set_call_id(&self.call_id);
        b.set_offer_kind(self.offer_kind);
        b.set_initiator_pubkey(&self.initiator_pubkey);
        b.set_initiator_x25519_pub(&self.initiator_x25519_pub);
        b.set_expires_at_ms(self.expires_at_ms);
        set_text_list(
            b.init_video_decode_codecs(list_len(&self.video_decode_codecs)),
            &self.video_decode_codecs,
        );
    }

    pub(super) fn read(r: call_invite::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            call_id: text(r.get_call_id())?,
            offer_kind: r.get_offer_kind(),
            initiator_pubkey: text(r.get_initiator_pubkey())?,
            initiator_x25519_pub: data(r.get_initiator_x25519_pub())?,
            expires_at_ms: r.get_expires_at_ms(),
            video_decode_codecs: text_list(r.get_video_decode_codecs())?,
        })
    }
}

/// 0x0201 CallRinging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRinging {
    pub call_id: String,
}

impl CallRinging {
    pub(super) fn write(&self, mut b: call_ringing::Builder<'_>) {
        b.set_call_id(&self.call_id);
    }

    pub(super) fn read(r: call_ringing::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            call_id: text(r.get_call_id())?,
        })
    }
}

/// 0x0202 CallAccept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallAccept {
    pub call_id: String,
    pub acceptor_x25519_pub: Vec<u8>,
    pub video_decode_codecs: Vec<String>,
}

impl CallAccept {
    pub(super) fn write(&self, mut b: call_accept::Builder<'_>) {
        b.set_call_id(&self.call_id);
        b.set_acceptor_x25519_pub(&self.acceptor_x25519_pub);
        set_text_list(
            b.init_video_decode_codecs(list_len(&self.video_decode_codecs)),
            &self.video_decode_codecs,
        );
    }

    pub(super) fn read(r: call_accept::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            call_id: text(r.get_call_id())?,
            acceptor_x25519_pub: data(r.get_acceptor_x25519_pub())?,
            video_decode_codecs: text_list(r.get_video_decode_codecs())?,
        })
    }
}

/// The body of 0x0203 CallDecline, 0x0204 CallEnd and 0x0212
/// GroupCallDecline: the call and why it closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallClosed {
    pub call_id: String,
    pub reason: String,
}

impl CallClosed {
    pub(super) fn write(&self, mut b: call_closed::Builder<'_>) {
        b.set_call_id(&self.call_id);
        b.set_reason(&self.reason);
    }

    pub(super) fn read(r: call_closed::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            call_id: text(r.get_call_id())?,
            reason: text(r.get_reason())?,
        })
    }
}

/// 0x0205 CallMediaState.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallMediaState {
    pub call_id: String,
    pub audio: bool,
    pub video: bool,
    pub screen: bool,
    pub timestamp_ms: u64,
}

impl CallMediaState {
    pub(super) fn write(&self, mut b: call_media_state::Builder<'_>) {
        b.set_call_id(&self.call_id);
        b.set_audio(self.audio);
        b.set_video(self.video);
        b.set_screen(self.screen);
        b.set_timestamp_ms(self.timestamp_ms);
    }

    pub(super) fn read(r: call_media_state::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            call_id: text(r.get_call_id())?,
            audio: r.get_audio(),
            video: r.get_video(),
            screen: r.get_screen(),
            timestamp_ms: r.get_timestamp_ms(),
        })
    }
}

/// 0x0206 CallReaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallReaction {
    pub call_id: String,
    pub emoji: String,
    pub timestamp_ms: u64,
}

impl CallReaction {
    pub(super) fn write(&self, mut b: call_reaction::Builder<'_>) {
        b.set_call_id(&self.call_id);
        b.set_emoji(&self.emoji);
        b.set_timestamp_ms(self.timestamp_ms);
    }

    pub(super) fn read(r: call_reaction::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            call_id: text(r.get_call_id())?,
            emoji: text(r.get_emoji())?,
            timestamp_ms: r.get_timestamp_ms(),
        })
    }
}

/// 0x0210 GroupCallOffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupCallOffer {
    pub call_id: String,
    pub offer_kind: u8,
    pub initiator_pubkey: String,
    pub initiator_x25519_pub: Vec<u8>,
    pub participants: Vec<String>,
    pub wrapped_call_key: Vec<u8>,
    pub expires_at_ms: u64,
}

impl GroupCallOffer {
    pub(super) fn write(&self, mut b: group_call_offer::Builder<'_>) {
        b.set_call_id(&self.call_id);
        b.set_offer_kind(self.offer_kind);
        b.set_initiator_pubkey(&self.initiator_pubkey);
        b.set_initiator_x25519_pub(&self.initiator_x25519_pub);
        set_text_list(
            b.reborrow().init_participants(list_len(&self.participants)),
            &self.participants,
        );
        b.set_wrapped_call_key(&self.wrapped_call_key);
        b.set_expires_at_ms(self.expires_at_ms);
    }

    pub(super) fn read(r: group_call_offer::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            call_id: text(r.get_call_id())?,
            offer_kind: r.get_offer_kind(),
            initiator_pubkey: text(r.get_initiator_pubkey())?,
            initiator_x25519_pub: data(r.get_initiator_x25519_pub())?,
            participants: text_list(r.get_participants())?,
            wrapped_call_key: data(r.get_wrapped_call_key())?,
            expires_at_ms: r.get_expires_at_ms(),
        })
    }
}

/// The body of 0x0211 GroupCallAccept (`pubkey` = the acceptor) and
/// 0x0213 GroupCallParticipantJoined (`pubkey` = the participant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupCallMember {
    pub call_id: String,
    pub pubkey: String,
}

impl GroupCallMember {
    pub(super) fn write(&self, mut b: group_call_member::Builder<'_>) {
        b.set_call_id(&self.call_id);
        b.set_pubkey(&self.pubkey);
    }

    pub(super) fn read(r: group_call_member::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            call_id: text(r.get_call_id())?,
            pubkey: text(r.get_pubkey())?,
        })
    }
}

/// 0x0214 GroupCallParticipantLeft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupCallParticipantLeft {
    pub call_id: String,
    pub participant_pubkey: String,
    pub reason: String,
}

impl GroupCallParticipantLeft {
    pub(super) fn write(&self, mut b: group_call_participant_left::Builder<'_>) {
        b.set_call_id(&self.call_id);
        b.set_participant_pubkey(&self.participant_pubkey);
        b.set_reason(&self.reason);
    }

    pub(super) fn read(r: group_call_participant_left::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            call_id: text(r.get_call_id())?,
            participant_pubkey: text(r.get_participant_pubkey())?,
            reason: text(r.get_reason())?,
        })
    }
}

/// 0x0220 DmVideoFragment, until E4.3.8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmVideoFragment {
    pub stream_id: [u8; 16],
    pub frame_seq: u32,
    pub fragment_index: u16,
    pub fragment_count: u16,
    pub keyframe: bool,
    /// "vp9" | "vp8" | "h264".
    pub codec: String,
    pub timestamp: u32,
    pub chunk: Vec<u8>,
}

impl DmVideoFragment {
    pub(super) fn write(&self, mut b: dm_video_fragment::Builder<'_>) {
        b.set_stream_id(&self.stream_id);
        b.set_frame_seq(self.frame_seq);
        b.set_fragment_index(self.fragment_index);
        b.set_fragment_count(self.fragment_count);
        b.set_keyframe(self.keyframe);
        b.set_codec(&self.codec);
        b.set_timestamp(self.timestamp);
        b.set_chunk(&self.chunk);
    }

    pub(super) fn read(r: dm_video_fragment::Reader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            stream_id: fixed(r.get_stream_id(), "video stream id")?,
            frame_seq: r.get_frame_seq(),
            fragment_index: r.get_fragment_index(),
            fragment_count: r.get_fragment_count(),
            keyframe: r.get_keyframe(),
            codec: text(r.get_codec())?,
            timestamp: r.get_timestamp(),
            chunk: data(r.get_chunk())?,
        })
    }
}
