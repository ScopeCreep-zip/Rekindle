//! Failure code registry — every named protocol failure.
//!
//! Codes are `u32` organized by category in the high byte.

/// Failure codes carried in NACK, STREAM_NACK, DATAGRAM_REJECT, and CHANNEL_ERROR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum FailureCode {
    // ── Category 0x0001: Handshake ─────────────────────────────────
    HandshakeTimeout           = 0x0001_0001,
    HandshakeNoiseFailed       = 0x0001_0002,
    HandshakeTranscriptMismatch = 0x0001_0003,
    PeerUnregistered           = 0x0001_0004,
    PeerKeyMismatch            = 0x0001_0005,
    PeerUidDisallowed          = 0x0001_0006,
    PeerClearanceMismatch      = 0x0001_0007,
    CapabilityMismatch         = 0x0001_0008,
    WireVersionUnsupported     = 0x0001_0009,
    HandshakeInvalidCredentials = 0x0001_000A,

    // ── Category 0x0002: Cryptographic ─────────────────────────────
    AeadVerificationFailed     = 0x0002_0001,
    NonceExhausted             = 0x0002_0002,
    KeyRotationAborted         = 0x0002_0003,
    KeyRotationTimeout         = 0x0002_0004,
    GenerationConflict         = 0x0002_0005,
    RevokedKeyPresented        = 0x0002_0006,
    AuditChainDivergence       = 0x0002_0007,
    AuditReplayGraftingFailed  = 0x0002_0008,
    AuditGapUnfillable         = 0x0002_0009,
    AuditLogTruncated          = 0x0002_000A,
    AuditKeyMismatch           = 0x0002_000B,
    EnvelopeMacFailed          = 0x0002_000C,
    HeaderMacFailed            = 0x0002_000D,

    // ── Category 0x0003: Protocol structure ────────────────────────
    FrameMalformed             = 0x0003_0001,
    FrameTooLarge              = 0x0003_0002,
    FrameClassUnknown          = 0x0003_0003,
    FrameKindUnknown           = 0x0003_0004,
    FrameClassLaneMismatch     = 0x0003_0005,
    FrameDisallowedInState     = 0x0003_0006,
    SequenceWraparound         = 0x0003_0007,
    ReservedBitSet             = 0x0003_0008,
    LaneUnknown                = 0x0003_0009,
    SequenceNonMonotonic       = 0x0003_000A,

    // ── Category 0x0004: Auth / authz ──────────────────────────────
    ClearanceInsufficient      = 0x0004_0001,
    ConditionViolation         = 0x0004_0002,
    TopicUnauthorized          = 0x0004_0003,
    TopicUnknown               = 0x0004_0004,
    RouteDenied                = 0x0004_0005,
    ReplayDetected             = 0x0004_0006,

    // ── Category 0x0005: Resource exhaustion ───────────────────────
    BackpressureExhausted      = 0x0005_0001,
    BudgetExhausted            = 0x0005_0002,
    PoolExhausted              = 0x0005_0003,
    PendingRequestsExhausted   = 0x0005_0004,
    StreamIdExhausted          = 0x0005_0005,
    SubscriptionQuotaExhausted = 0x0005_0006,
    SubscriptionSlowConsumer   = 0x0005_0007,
    RateLimited                = 0x0005_0008,

    // ── Category 0x0006: Application coordination ──────────────────
    AckTimeout                 = 0x0006_0001,
    ReplyTimeout               = 0x0006_0002,
    HeartbeatTimeout           = 0x0006_0003,
    DrainTimeout               = 0x0006_0004,
    QuiescenceTimeout          = 0x0006_0005,
    ResumeWindowExpired        = 0x0006_0006,
    AuditAnchorMismatch        = 0x0006_0007,
    ContentAnchorMismatch      = 0x0006_0008,
    ContentHashMismatch        = 0x0006_0009,
    TransferIdUnknown          = 0x0006_000A,
    LineageMismatch            = 0x0006_000B,
    StreamAlreadyOpen          = 0x0006_000C,
    DedupCacheMiss             = 0x0006_000D,
    DedupCacheMismatch         = 0x0006_000E,
    DedupClearanceInsufficient = 0x0006_000F,
    HandoffContentHashMismatch = 0x0006_0010,
    HandoffSizeMismatch        = 0x0006_0011,
    HandoffMacFailed           = 0x0006_0012,
    HandoffFdMissing           = 0x0006_0013,
    HandoffTimeout             = 0x0006_0014,

    // ── Category 0x0007: Substrate ─────────────────────────────────
    SubstrateReadFailed        = 0x0007_0001,
    SubstrateWriteFailed       = 0x0007_0002,
    SubstrateEof               = 0x0007_0003,
    SubstrateBrokenPipe        = 0x0007_0004,
    SubstrateConnectionReset   = 0x0007_0005,
    SubstrateAllocationFailed  = 0x0007_0006,
    PeerCrashed                = 0x0007_0007,
    ConnectionLost             = 0x0007_0008,
    ConnectionLostQuiesced     = 0x0007_0009,
}

impl FailureCode {
    /// Category of a failure code (high 16 bits).
    pub fn category(self) -> u16 {
        (self as u32 >> 16) as u16
    }
}
