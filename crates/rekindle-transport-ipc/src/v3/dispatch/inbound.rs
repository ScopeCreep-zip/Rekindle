//! The single dispatch function. Every inbound frame routes through here.
//! Validates class↔lane coupling, then calls the real handler which mutates
//! SessionContext, delivers to FrameRouter, and pushes outbound responses.

use crate::v3::codec::envelope::EnvelopeInfo;
use crate::v3::codec::header::StreamHeaderInfo;
use crate::v3::context::SessionContext;
use crate::v3::handlers::{self, HandlerError};
use crate::v3::wire::lane::Lane;

#[derive(Debug)]
pub enum ProtocolError {
    UnknownFrame { class: u8, kind: u8 },
    FrameClassLaneMismatch { lane: Lane, class: u8 },
    PayloadTooShort,
    Handler(HandlerError),
}

impl From<HandlerError> for ProtocolError {
    fn from(e: HandlerError) -> Self {
        Self::Handler(e)
    }
}

impl ProtocolError {
    pub fn failure_code(&self) -> crate::v3::wire::failure::FailureCode {
        use crate::v3::wire::failure::FailureCode;
        match self {
            Self::UnknownFrame { .. } => FailureCode::FrameKindUnknown,
            Self::FrameClassLaneMismatch { .. } => FailureCode::FrameClassLaneMismatch,
            Self::PayloadTooShort => FailureCode::FrameMalformed,
            Self::Handler(e) => e.failure_code(),
        }
    }
}

pub fn dispatch_frame(
    ctx: &mut SessionContext,
    envelope: &EnvelopeInfo,
    header: Option<&StreamHeaderInfo>,
    payload: &[u8],
) -> Result<(), ProtocolError> {
    if payload.len() < 2 {
        return Err(ProtocolError::PayloadTooShort);
    }

    let (class_byte, kind_byte) = if let Some(hdr) = header {
        (hdr.frame_class as u8, hdr.frame_kind as u8)
    } else {
        (payload[0], payload[1])
    };

    tracing::debug!(
        session_seq = envelope.session_seq,
        lane = ?envelope.lane,
        class = class_byte,
        kind = kind_byte,
        state = ?ctx.session_state(),
        "dispatch_frame: routing inbound frame"
    );

    let class_valid = matches!(
        (envelope.lane, class_byte),
        (Lane::Control, 0x01 | 0x03)
            | (Lane::Data, 0x02)
            | (Lane::Audit, 0x04)
            | (Lane::Handoff, 0x05)
    );
    if !class_valid {
        tracing::error!(
            lane = ?envelope.lane, class = class_byte, kind = kind_byte,
            state = ?ctx.session_state(),
            "dispatch_frame: REJECTED — class/lane mismatch"
        );
        return Err(ProtocolError::FrameClassLaneMismatch {
            lane: envelope.lane,
            class: class_byte,
        });
    }

    // Session state gate — reject frames not permitted in the current state
    use crate::v3::session::state::is_frame_allowed;
    use crate::v3::wire::frame_class::FrameClass;
    if let Ok(frame_class) = FrameClass::try_from(class_byte) {
        if !is_frame_allowed(ctx.session_state(), frame_class, kind_byte) {
            tracing::debug!(
                state = ?ctx.session_state(),
                class = class_byte,
                kind = kind_byte,
                session_seq = envelope.session_seq,
                "dispatch_frame: REJECTED — frame disallowed in state"
            );
            return Err(ProtocolError::Handler(HandlerError::FrameDisallowedInState));
        }
    }

    // For non-Data lanes, plaintext starts with [class, kind, ...] — skip 2.
    // For Data Lane, class/kind are in the Stream Header, not the plaintext.
    // The plaintext IS the application payload — no prefix to skip.
    let handler_payload = if header.is_some() {
        payload
    } else {
        &payload[2..]
    };

    let result = match (class_byte, kind_byte) {
        // ── Channel (0x01) ────────────────────────────────────
        (0x01, 0x01) => Err(HandlerError::FrameDisallowedInState),
        (0x01, 0x02) => Err(HandlerError::FrameDisallowedInState),
        (0x01, 0x03) => handlers::channel::goodbye::handle(ctx, handler_payload),
        (0x01, 0x04) => handlers::channel::goodbye_ack::handle(ctx, handler_payload),
        (0x01, 0x05) => handlers::channel::ping::handle(ctx, handler_payload),
        (0x01, 0x06) => handlers::channel::pong::handle(ctx, handler_payload),
        (0x01, 0x07) => handlers::channel::ack::handle(ctx, handler_payload),
        (0x01, 0x08) => handlers::channel::nack::handle(ctx, handler_payload),
        (0x01, 0x09) => handlers::channel::credit::handle(ctx, handler_payload),
        (0x01, 0x0A) => handlers::channel::backpressure::handle_assert(ctx, handler_payload),
        (0x01, 0x0B) => { handlers::channel::backpressure::handle_clear(ctx, handler_payload); Ok(()) },
        (0x01, 0x0C) => handlers::channel::rotate::handle_init(ctx, handler_payload),
        (0x01, 0x0D) => handlers::channel::rotate::handle_commit(ctx, handler_payload),
        (0x01, 0x0E) => handlers::channel::revoke::handle(ctx, handler_payload),
        (0x01, 0x0F) => handlers::channel::error::handle(ctx, handler_payload),
        (0x01, 0x10) => handlers::channel::subscribe::handle(ctx, handler_payload),
        (0x01, 0x11) => Ok(()),
        (0x01, 0x12) => Ok(()),
        (0x01, 0x13) => handlers::channel::unsubscribe::handle(ctx, handler_payload),
        (0x01, 0x14) => Ok(()),
        (0x01, 0x15) => Ok(()),
        (0x01, 0x16) => Ok(()),
        (0x01, 0x17) => Ok(()),
        (0x01, 0x18) => handlers::channel::quiesce::handle_quiesce(ctx, handler_payload),
        (0x01, 0x19) => Ok(()),
        (0x01, 0x1A) => handlers::channel::quiesce::handle_resume(ctx, handler_payload),
        (0x01, 0x1B) => Ok(()),
        (0x01, 0x1C) => Ok(()),

        // ── Stream (0x02) ─────────────────────────────────────
        (0x02, 0x01) => {
            let h = header.ok_or(HandlerError::CodecFailed("missing stream header".into()))?;
            handlers::stream::open::handle(ctx, h, handler_payload)
        }
        (0x02, 0x02) => {
            let h = header.ok_or(HandlerError::CodecFailed("missing stream header".into()))?;
            handlers::stream::payload::handle(ctx, h, handler_payload)
        }
        (0x02, 0x03) => {
            let h = header.ok_or(HandlerError::CodecFailed("missing stream header".into()))?;
            handlers::stream::fault::handle(ctx, h, handler_payload)
        }
        (0x02, 0x04) => {
            let h = header.ok_or(HandlerError::CodecFailed("missing stream header".into()))?;
            handlers::stream::fin::handle(ctx, h, handler_payload)
        }
        (0x02, 0x05) => {
            let h = header.ok_or(HandlerError::CodecFailed("missing stream header".into()))?;
            handlers::stream::ack::handle(ctx, h, handler_payload)
        }
        (0x02, 0x06) => {
            let h = header.ok_or(HandlerError::CodecFailed("missing stream header".into()))?;
            handlers::stream::nack::handle(ctx, h, handler_payload)
        }
        (0x02, 0x07) => {
            let h = header.ok_or(HandlerError::CodecFailed("missing stream header".into()))?;
            handlers::stream::reset::handle(ctx, h, handler_payload)
        }
        (0x02, 0x08) => {
            let h = header.ok_or(HandlerError::CodecFailed("missing stream header".into()))?;
            handlers::stream::cancel::handle(ctx, h, handler_payload)
        }
        (0x02, 0x09) => Ok(()),
        (0x02, 0x0A) => {
            let h = header.ok_or(HandlerError::CodecFailed("missing stream header".into()))?;
            handlers::stream::resume::handle(ctx, h, handler_payload)
        }
        (0x02, 0x0B) => Ok(()),
        (0x02, 0x0C) => {
            let h = header.ok_or(HandlerError::CodecFailed("missing stream header".into()))?;
            handlers::stream::credit::handle(ctx, h, handler_payload)
        }
        (0x02, 0x0D) => {
            let h = header.ok_or(HandlerError::CodecFailed("missing stream header".into()))?;
            handlers::stream::reference::handle(ctx, h, handler_payload)
        }
        (0x02, 0x0E) => {
            let h = header.ok_or(HandlerError::CodecFailed("missing stream header".into()))?;
            handlers::stream::sack::handle(ctx, h, handler_payload)
        }

        // ── Datagram (0x03) ───────────────────────────────────
        (0x03, 0x01) => handlers::datagram::request::handle(ctx, handler_payload),
        (0x03, 0x02) => handlers::datagram::reply::handle(ctx, handler_payload),
        (0x03, 0x03) => handlers::datagram::notify::handle(ctx, handler_payload),
        (0x03, 0x04) => handlers::datagram::publish::handle(ctx, handler_payload),
        (0x03, 0x05) => handlers::datagram::reject::handle(ctx, handler_payload),

        // ── Audit (0x04) ──────────────────────────────────────
        (0x04, 0x01) => handlers::audit::checkpoint::handle(ctx, handler_payload),
        (0x04, 0x02) => handlers::audit::query::handle(ctx, handler_payload),
        (0x04, 0x03) => handlers::audit::proof::handle(ctx, handler_payload),
        (0x04, 0x04) => handlers::audit::gap::handle(ctx, handler_payload),
        (0x04, 0x05) => handlers::audit::replay::handle(ctx, handler_payload),

        // ── Handoff (0x05) ────────────────────────────────────
        (0x05, 0x01) => handlers::handoff::offer::handle(ctx, handler_payload),
        (0x05, 0x02) => handlers::handoff::accept::handle(ctx, handler_payload),
        (0x05, 0x03) => handlers::handoff::reject::handle(ctx, handler_payload),
        (0x05, 0x04) => handlers::handoff::revoke::handle(ctx, handler_payload),
        (0x05, 0x05) => handlers::handoff::confirm::handle(ctx, handler_payload),

        (c, k) => return Err(ProtocolError::UnknownFrame { class: c, kind: k }),
    };

    result.map_err(ProtocolError::Handler)
}
