//! A set on [`SimDht`]: the value it sends and its fanout over the
//! replicas, at a seq of at least the call's `min_seqnum` when given
//! ([`set_above`](rekindle_records::port::DhtBackend::set_above)).

use rekindle_records::port::{DhtCallError, DhtErrorKind, RecordValue, WriterKey};

use super::super::{fanout, public_text, replicas, writer_public};
use super::{error, try_again, SimDht};
use crate::latency;

impl SimDht {
    /// A set of `data` as `writer`, at a seq of at least `min_seq`.
    pub(super) async fn write(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&WriterKey>,
        min_seq: Option<u32>,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        self.net.inner.lock().nodes[self.node].app_sets += 1;
        let opened = self.opened(key)?;
        let writer = writer
            .map(|w| w.keypair.clone())
            .or(opened)
            .ok_or_else(|| error(DhtErrorKind::Generic, "value is not writable"))?;
        let public =
            writer_public(&writer).map_err(|e| error(DhtErrorKind::ParseError, e.to_string()))?;
        self.validate(key, subkey, &public, data.len())?;
        // In flight until it returns (0.5.8 `inspect_record.rs:274-277,287-290`).
        let _setting = super::super::Setting::new(&self.net, self.node, key, subkey);
        if self.offline() {
            return Err(try_again());
        }
        let _slot = self.net.latency.slot(self.node, key, &[subkey]).await;
        if self.fault_try_again() {
            tokio::time::sleep(self.net.latency.draw(self.node, latency::SET)).await;
            return Err(try_again());
        }
        let accepting = {
            let inner = self.net.inner.lock();
            inner
                .faults
                .accepting(self.node, key, subkey, inner.records.get(key))
        };
        let ours = self.ours(key, subkey, data, &public, min_seq);
        let fanout = fanout::run(&self.net, self.node, (key, subkey), &ours, accepting).await;
        if self.offline() {
            return Err(try_again());
        }
        fanout::commit(&self.net, self.node, (key, subkey), &ours, &fanout)
    }

    /// The value a write sends (0.5.8 `set_value.rs:353-386`): the same data
    /// and writer resend the local copy's bytes at its seq, else a fresh
    /// encryption at local seq + 1, or seq 0 with no local copy; a
    /// `min_seqnum` (the call's [`set_above`](rekindle_records::port::DhtBackend::set_above), or a modified
    /// client's, [`crate::Fault::MinSeqnum`]) raises the seq to it, and an
    /// identical value it lifts is encrypted afresh ([`replicas::encrypt`]).
    fn ours(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        public: &[u8; 32],
        min_seq: Option<u32>,
    ) -> replicas::Cipher {
        let writer = public_text(public);
        let mut inner = self.net.inner.lock();
        let min = inner
            .faults
            .min_seqnum(self.node, key)
            .max(min_seq.unwrap_or(0));
        let seq = match replicas::local(&inner, self.node, key, subkey) {
            Some(l) if l.value.data == data && l.value.writer == writer => {
                if min <= l.value.seq {
                    return l;
                }
                l.value.seq
            }
            Some(l) => l.value.seq.saturating_add(1),
            None => 0,
        };
        replicas::encrypt(
            &mut inner,
            RecordValue {
                seq: seq.max(min),
                data,
                writer,
            },
        )
    }
}
