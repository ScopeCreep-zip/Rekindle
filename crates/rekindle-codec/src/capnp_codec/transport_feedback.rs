//! `TransportFeedback` (schemas/media_feedback.capnp): the receiver's
//! per-route report of which media datagrams arrived and when (plan
//! E4.3.2, RFC 8888-shaped). Signed by the receiver, so only a peer can
//! report on its own reception.

use rekindle_types::domains::TRANSPORT_FEEDBACK;

use super::{bounds, capnp_err, pack, unpack_bounded, CodecError};
use crate::media_feedback_capnp::transport_feedback;

/// An `arrivals` entry: the datagram did not arrive.
pub const NOT_RECEIVED: u16 = 0xFFFF;
/// The largest arrival offset an entry carries, in 1/1024 s; older
/// arrivals are clamped to it.
pub const MAX_ARRIVAL_OFFSET: u16 = 0xFFFE;

/// One transport feedback report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportFeedback {
    /// The reporting receiver's signing key (32 bytes).
    pub reporter_key: Vec<u8>,
    /// First transport sequence number covered.
    pub begin_seq: u32,
    /// Receiver clock, milliseconds, when the report was built.
    pub report_time_ms: u64,
    /// Per sequence number from `begin_seq`: [`NOT_RECEIVED`], or the
    /// arrival offset before `report_time_ms` in 1/1024 s.
    pub arrivals: Vec<u16>,
    /// Ed25519 signature over [`Self::signing_bytes`].
    pub sig: Vec<u8>,
}

impl TransportFeedback {
    /// The bytes the receiver signs: the `TRANSPORT_FEEDBACK` domain, then
    /// every field but the signature.
    #[must_use]
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            TRANSPORT_FEEDBACK.len() + self.reporter_key.len() + 12 + self.arrivals.len() * 2,
        );
        out.extend_from_slice(TRANSPORT_FEEDBACK.as_bytes());
        out.extend_from_slice(&self.reporter_key);
        out.extend_from_slice(&self.begin_seq.to_le_bytes());
        out.extend_from_slice(&self.report_time_ms.to_le_bytes());
        for a in &self.arrivals {
            out.extend_from_slice(&a.to_le_bytes());
        }
        out
    }

    /// Packed Cap'n Proto bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut builder = capnp::message::Builder::new_default();
        {
            let mut root = builder.init_root::<transport_feedback::Builder<'_>>();
            root.set_reporter_key(&self.reporter_key);
            root.set_begin_seq(self.begin_seq);
            root.set_report_time_ms(self.report_time_ms);
            let len = u32::try_from(self.arrivals.len()).unwrap_or(u32::MAX);
            let mut list = root.reborrow().init_arrivals(len);
            for (i, a) in self.arrivals.iter().enumerate().take(len as usize) {
                list.set(u32::try_from(i).unwrap_or(u32::MAX), *a);
            }
            root.set_sig(&self.sig);
        }
        pack(&builder)
    }

    /// Decode packed Cap'n Proto bytes. The signature is not checked.
    ///
    /// # Errors
    /// The bytes are not a `TransportFeedback`.
    pub fn decode(data: &[u8]) -> Result<Self, CodecError> {
        let reader = unpack_bounded(data, bounds::TRANSPORT_FEEDBACK_BOUND)?;
        let root = reader
            .get_root::<transport_feedback::Reader<'_>>()
            .map_err(|e| capnp_err(&e))?;
        let list = root.get_arrivals().map_err(|e| capnp_err(&e))?;
        Ok(Self {
            reporter_key: root.get_reporter_key().map_err(|e| capnp_err(&e))?.to_vec(),
            begin_seq: root.get_begin_seq(),
            report_time_ms: root.get_report_time_ms(),
            arrivals: list.iter().collect(),
            sig: root.get_sig().map_err(|e| capnp_err(&e))?.to_vec(),
        })
    }
}

impl super::SignedWire for TransportFeedback {
    const WHAT: &'static str = "transport feedback";
    fn signing_bytes(&self) -> Vec<u8> {
        TransportFeedback::signing_bytes(self)
    }
    fn signer_key(&self) -> &[u8] {
        &self.reporter_key
    }
    fn signature(&self) -> &[u8] {
        &self.sig
    }
    fn set_signature(&mut self, sig: Vec<u8>) {
        self.sig = sig;
    }
}

#[cfg(test)]
#[path = "transport_feedback_tests.rs"]
mod tests;
