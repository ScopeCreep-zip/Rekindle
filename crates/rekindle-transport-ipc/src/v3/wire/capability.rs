//! Capability bitfield — negotiated at handshake, immutable for Session lifetime.

bitflags::bitflags! {
    /// 64-bit capability bitmask. Active set = bitwise AND of both peers.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CapabilityBits: u64 {
        const AEAD_AES256GCM          = 1 << 0;
        const AEAD_CHACHA20POLY1305   = 1 << 1;
        const AEAD_AEGIS128L          = 1 << 2;
        const AEAD_AES256GCMSIV       = 1 << 3;
        const HASH_BLAKE3             = 1 << 4;
        const HASH_SHA256             = 1 << 5;
        const HANDOFF_MEMFD           = 1 << 6;
        const HANDOFF_PIPE            = 1 << 7;
        const DEDUP_CACHE             = 1 << 8;
        const DEDUP_CROSS_SESSION     = 1 << 9;
        const AUDIT_CHAIN             = 1 << 10;
        const AUDIT_REPLAY            = 1 << 11;
        const AUDIT_PROOF             = 1 << 12;
        const QUIESCENCE              = 1 << 13;
        const RESUME                  = 1 << 14;
        const FLOW_CREDIT             = 1 << 15;
        const CONDITIONS              = 1 << 16;
        const CLEARANCE_MULTI_TIER    = 1 << 17;
        const SUBSCRIPTION            = 1 << 18;
        const SUBSCRIPTION_CONDITIONS = 1 << 19;
        const SUBSCRIPTION_MULTI_TOPIC = 1 << 20;
        const BATCHED_ACK             = 1 << 21;
        const SUPPRESSED_ACK          = 1 << 22;
        const LANE_CONTROL            = 1 << 23;
        const LANE_DATA               = 1 << 24;
        const LANE_AUDIT              = 1 << 25;
        const LANE_HANDOFF            = 1 << 26;
        const FIPS_MODE               = 1 << 27;
        const POSTQUANTUM_HYBRID      = 1 << 28;
        const EBPF_FASTPATH           = 1 << 29;
        const KEY_ROTATION            = 1 << 30;
        const LINEAGE                 = 1 << 31;
        const SACK                    = 1 << 32;
        const SIDECHANNEL_CREDIT      = 1 << 33;
    }
}

impl CapabilityBits {
    /// The minimum set every v1 peer must advertise.
    pub const MANDATORY_V1: CapabilityBits = Self::AEAD_AES256GCM
        .union(Self::HASH_BLAKE3)
        .union(Self::LANE_CONTROL)
        .union(Self::LANE_DATA)
        .union(Self::AUDIT_CHAIN)
        .union(Self::FLOW_CREDIT)
        .union(Self::BATCHED_ACK)
        .union(Self::KEY_ROTATION)
        .union(Self::SACK)
        .union(Self::SIDECHANNEL_CREDIT);

    /// Bits 34..63 are reserved. This mask covers them.
    pub const RESERVED_MASK: u64 = !((1u64 << 34) - 1);
}
