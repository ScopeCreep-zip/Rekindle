//! Transport-wide sequence numbers and send records.
//!
//! Copied from str0m `src/rtp/id.rs` (`TwccSeq`, `TwccClusterId`) and
//! `src/rtp/rtcp/twcc.rs` (`TwccPacketId`, `TwccSendRecord`, `TwccRecvReport`).
//! The RTCP TWCC parser and `TwccSendRegister` are not copied: Rekindle's
//! transport feedback builds the send records itself, through
//! [`TwccSendRecord::new`] and [`TwccRecvReport::new`].

use std::fmt;
use std::ops::Deref;
use std::time::{Duration, Instant};

/// TWCC-specific sequence number.
///
/// Transport-Wide Congestion Control uses its own sequence number space,
/// separate from RTP sequence numbers. This type ensures TWCC sequences
/// cannot be confused with RTP SeqNo values.
///
/// TWCC sequences are also u64 internally (tracking rollovers), though the
/// wire format uses u16.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct TwccSeq(u64);
num_id!(TwccSeq, u64);

impl TwccSeq {
    /// Check if the `other` sequence number is directly following this.
    #[inline]
    pub fn is_next(&self, other: TwccSeq) -> bool {
        if **self >= *other {
            return false;
        }
        *other - **self == 1
    }

    /// Increase (mutate) this sequence number and return the previous value.
    #[inline]
    pub fn inc(&mut self) -> TwccSeq {
        let n = TwccSeq(self.0);
        self.0 += 1;
        n
    }

    /// The TWCC wire format value (discarding the ROC).
    ///
    /// This is the same as discarding the top 48 bits by casting to a u16.
    #[inline]
    pub fn as_u16(&self) -> u16 {
        u16::try_from(self.0 & u64::from(u16::MAX)).unwrap_or_default()
    }

    /// Get the rollover counter (ROC) value.
    #[inline]
    pub fn roc(&self) -> u64 {
        self.0 >> 16
    }
}

/// Probe cluster identifier for bandwidth estimation.
///
/// Used to tag TWCC packets as belonging to a specific probe cluster,
/// enabling analysis of probe results when feedback arrives.
///
/// Uses u64 to avoid wrap-around in long-running connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TwccClusterId(u64);
num_id!(TwccClusterId, u64);

impl TwccClusterId {
    /// Increase (mutate) this cluster ID and return the previous value.
    #[inline]
    pub fn inc(&mut self) -> TwccClusterId {
        let n = TwccClusterId(self.0);
        self.0 = self.0.wrapping_add(1);
        n
    }
}

/// Packet identification for TWCC tracking.
///
/// Groups together the TWCC sequence number and optional probe cluster context.
#[derive(Debug, Clone, Copy)]
pub struct TwccPacketId {
    /// TWCC sequence number
    seq: TwccSeq,
    /// Probe cluster this packet belongs to, if any
    cluster: Option<TwccClusterId>,
}

impl TwccPacketId {
    /// Create a packet ID for a regular media packet (no probe cluster).
    pub fn new(seq: impl Into<TwccSeq>) -> Self {
        Self {
            seq: seq.into(),
            cluster: None,
        }
    }

    /// Create a packet ID for a probe packet.
    pub fn with_cluster(seq: impl Into<TwccSeq>, cluster: impl Into<TwccClusterId>) -> Self {
        Self {
            seq: seq.into(),
            cluster: Some(cluster.into()),
        }
    }

    /// Get the TWCC sequence number.
    pub fn seq(&self) -> TwccSeq {
        self.seq
    }

    /// Get the probe cluster, if any.
    pub fn cluster(&self) -> Option<TwccClusterId> {
        self.cluster
    }
}

/// Record for a send entry in twcc.
#[derive(Debug)]
pub struct TwccSendRecord {
    /// Packet identification (sequence + optional probe cluster)
    packet_id: TwccPacketId,

    /// The (local) time we sent the packet represented by seq.
    local_send_time: Instant,

    /// Size in bytes of the payload sent.
    size: u16,

    recv_report: Option<TwccRecvReport>,
}

impl TwccSendRecord {
    /// Build a send record from the sender's own bookkeeping plus, once feedback
    /// has arrived, the receive report for the packet.
    ///
    /// Replaces str0m's `TwccSendRegister::register_seq` + `apply_report`: the
    /// caller keeps the send time and size, and fills `recv_report` from its
    /// transport feedback. A report whose `remote_recv_time` is `None` marks the
    /// packet as lost.
    ///
    /// `size` is the payload size in bytes. As in str0m it is stored as `u16`
    /// (the MTU bounds it); larger values saturate at `u16::MAX`.
    pub fn new(
        packet_id: TwccPacketId,
        local_send_time: Instant,
        size: usize,
        recv_report: Option<TwccRecvReport>,
    ) -> Self {
        Self {
            packet_id,
            local_send_time,
            size: u16::try_from(size).unwrap_or(u16::MAX),
            recv_report,
        }
    }

    /// The twcc sequence number of the packet we sent.
    pub fn seq(&self) -> TwccSeq {
        self.packet_id.seq()
    }

    /// The probe cluster this packet belongs to, if it's a probe packet.
    pub fn cluster(&self) -> Option<TwccClusterId> {
        self.packet_id.cluster()
    }

    /// The time we sent the packet.
    pub fn local_send_time(&self) -> Instant {
        self.local_send_time
    }

    /// The time we received this TWCC record. [`None`] if no feedback has been received yet.
    pub fn local_recv_time(&self) -> Option<Instant> {
        self.recv_report.as_ref().map(|r| r.local_recv_time)
    }

    /// Size in bytes of the payload sent.
    pub fn size(&self) -> usize {
        usize::from(self.size)
    }

    /// The time indicated by the remote side for when they received the packet.
    pub fn remote_recv_time(&self) -> Option<Instant> {
        self.recv_report.as_ref().and_then(|r| r.remote_recv_time)
    }

    /// The rtt time between sending the packet and receiving the twcc report response.
    pub fn rtt(&self) -> Option<Duration> {
        let recv_report = self.recv_report.as_ref()?;
        Some(recv_report.local_recv_time - self.local_send_time)
    }
}

#[cfg(test)]
impl TwccSendRecord {
    /// Test-only constructor to build TWCC send records with arbitrary receive status.
    ///
    /// This is used by unit tests for BWE/probing, allowing them to model received vs lost packets
    /// without constructing full RTCP TWCC reports.
    pub(crate) fn test_new(
        packet_id: TwccPacketId,
        local_send_time: Instant,
        size: usize,
        local_recv_time: Instant,
        remote_recv_time: Option<Instant>,
    ) -> Self {
        Self::new(
            packet_id,
            local_send_time,
            size,
            Some(TwccRecvReport::new(local_recv_time, remote_recv_time)),
        )
    }
}

/// The receive status of one sent packet, as learned from feedback.
#[derive(Debug, Copy, Clone)]
pub struct TwccRecvReport {
    ///  The (local) time we received confirmation the other side received the seq.
    local_recv_time: Instant,

    /// The remote time the other side received the seq.
    remote_recv_time: Option<Instant>,
}

impl TwccRecvReport {
    /// A receive report.
    ///
    /// * `local_recv_time`: when the feedback carrying this packet's status arrived here.
    /// * `remote_recv_time`: when the remote received the packet, mapped into local
    ///   `Instant`s on a timeline anchored once per session (str0m anchors it at the
    ///   first feedback). Only differences between these instants are used. `None`
    ///   means the feedback reports the packet as not received.
    pub fn new(local_recv_time: Instant, remote_recv_time: Option<Instant>) -> Self {
        Self {
            local_recv_time,
            remote_recv_time,
        }
    }
}
