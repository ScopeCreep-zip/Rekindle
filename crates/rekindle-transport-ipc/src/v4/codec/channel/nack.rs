use crate::v4::wire::failure::FailureCode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NackPayload {
    pub reason_code: FailureCode,
    pub rejected_message_id: uuid::Uuid,
    pub session_seq_rejected: u64,
    pub detail: String,
}

pub fn encode(p: &NackPayload) -> Vec<u8> {
    let detail_bytes = p.detail.as_bytes();
    let mut buf = Vec::with_capacity(32 + detail_bytes.len());
    buf.extend_from_slice(&(p.reason_code as u32).to_le_bytes());
    crate::v4::codec::write_u32_len(&mut buf, detail_bytes.len());
    buf.extend_from_slice(p.rejected_message_id.as_bytes());
    buf.extend_from_slice(&p.session_seq_rejected.to_le_bytes());
    buf.extend_from_slice(detail_bytes);
    buf
}

pub fn decode(buf: &[u8]) -> Result<NackPayload, CodecError> {
    if buf.len() < 32 {
        return Err(CodecError::TooShort { expected: 32, got: buf.len() });
    }
    let reason_u32 = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    let reason_code = failure_code_from_u32(reason_u32)?;
    let detail_len = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    let rejected_message_id = uuid::Uuid::from_bytes(buf[8..24].try_into().unwrap());
    let session_seq_rejected = u64::from_le_bytes(buf[24..32].try_into().unwrap());

    if buf.len() < 32 + detail_len {
        return Err(CodecError::TooShort { expected: 32 + detail_len, got: buf.len() });
    }
    let detail = String::from_utf8(buf[32..32 + detail_len].to_vec())
        .map_err(|_| CodecError::InvalidUtf8)?;

    Ok(NackPayload {
        reason_code,
        rejected_message_id,
        session_seq_rejected,
        detail,
    })
}

pub fn failure_code_from_u32(v: u32) -> Result<FailureCode, CodecError> {
    // Safety: FailureCode is repr(u32). We transmute only known values.
    // For unknown values, return an error rather than UB.
    let all_codes: &[FailureCode] = &[
        FailureCode::HandshakeTimeout, FailureCode::HandshakeNoiseFailed,
        FailureCode::HandshakeTranscriptMismatch, FailureCode::PeerUnregistered,
        FailureCode::PeerKeyMismatch, FailureCode::PeerUidDisallowed,
        FailureCode::PeerClearanceMismatch, FailureCode::CapabilityMismatch,
        FailureCode::WireVersionUnsupported, FailureCode::HandshakeInvalidCredentials,
        FailureCode::AeadVerificationFailed, FailureCode::NonceExhausted,
        FailureCode::KeyRotationAborted, FailureCode::KeyRotationTimeout,
        FailureCode::GenerationConflict, FailureCode::RevokedKeyPresented,
        FailureCode::AuditChainDivergence, FailureCode::AuditReplayGraftingFailed,
        FailureCode::AuditGapUnfillable, FailureCode::AuditLogTruncated,
        FailureCode::AuditKeyMismatch, FailureCode::EnvelopeMacFailed,
        FailureCode::HeaderMacFailed,
        FailureCode::FrameMalformed, FailureCode::FrameTooLarge,
        FailureCode::FrameClassUnknown, FailureCode::FrameKindUnknown,
        FailureCode::FrameClassLaneMismatch, FailureCode::FrameDisallowedInState,
        FailureCode::SequenceWraparound, FailureCode::ReservedBitSet,
        FailureCode::LaneUnknown, FailureCode::SequenceNonMonotonic,
        FailureCode::ClearanceInsufficient, FailureCode::ConditionViolation,
        FailureCode::TopicUnauthorized, FailureCode::TopicUnknown,
        FailureCode::RouteDenied, FailureCode::ReplayDetected,
        FailureCode::BackpressureExhausted, FailureCode::BudgetExhausted,
        FailureCode::PoolExhausted, FailureCode::PendingRequestsExhausted,
        FailureCode::StreamIdExhausted, FailureCode::SubscriptionQuotaExhausted,
        FailureCode::SubscriptionSlowConsumer, FailureCode::RateLimited,
        FailureCode::AckTimeout, FailureCode::ReplyTimeout,
        FailureCode::HeartbeatTimeout, FailureCode::DrainTimeout,
        FailureCode::QuiescenceTimeout, FailureCode::ResumeWindowExpired,
        FailureCode::AuditAnchorMismatch, FailureCode::ContentAnchorMismatch,
        FailureCode::ContentHashMismatch, FailureCode::TransferIdUnknown,
        FailureCode::LineageMismatch, FailureCode::StreamAlreadyOpen,
        FailureCode::DedupCacheMiss, FailureCode::DedupCacheMismatch,
        FailureCode::DedupClearanceInsufficient,
        FailureCode::HandoffContentHashMismatch, FailureCode::HandoffSizeMismatch,
        FailureCode::HandoffMacFailed, FailureCode::HandoffFdMissing,
        FailureCode::HandoffTimeout,
        FailureCode::SubstrateReadFailed, FailureCode::SubstrateWriteFailed,
        FailureCode::SubstrateEof, FailureCode::SubstrateBrokenPipe,
        FailureCode::SubstrateConnectionReset, FailureCode::SubstrateAllocationFailed,
        FailureCode::PeerCrashed, FailureCode::ConnectionLost,
        FailureCode::ConnectionLostQuiesced,
    ];
    for &code in all_codes {
        if code as u32 == v {
            return Ok(code);
        }
    }
    Err(CodecError::InvalidFailureCode(v))
}

#[derive(Debug)]
pub enum CodecError {
    TooShort { expected: usize, got: usize },
    InvalidUtf8,
    InvalidFailureCode(u32),
}
