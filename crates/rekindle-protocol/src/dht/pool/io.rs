//! Reads, writes and inspects on leased records.

use rekindle_records::lease::{LeaseId, SubkeySet};
use veilid_core::{
    AllowOffline, DHTRecordReport, DHTReportScope, KeyPair, RecordKey, SetDHTValueOptions,
    ValueData, ValueSubkeyRangeSet, VeilidAPIError,
};

use super::{range, to_protocol, RecordPool};
use crate::ProtocolError;

use rekindle_records::port::SetOutcome;

impl RecordPool {
    /// Read a subkey; `force_refresh` asks the network rather than the local
    /// copy.
    ///
    /// # Errors
    /// The lease is not held, or the read failed within the retry budget.
    pub async fn get(
        &self,
        id: LeaseId,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<ValueData>, ProtocolError> {
        let (key, _) = self.lookup(id)?;
        self.run_retrying("dht get", move |rc| {
            let k = key.clone();
            async move { rc.get_dht_value(k, subkey, force_refresh).await }
        })
        .await
        .map_err(|e| to_protocol(&e))
    }

    /// Inspect the leased record.
    ///
    /// # Errors
    /// The lease is not held, or the inspect failed within the retry budget.
    pub async fn inspect(
        &self,
        id: LeaseId,
        subkeys: Option<SubkeySet>,
        scope: DHTReportScope,
    ) -> Result<DHTRecordReport, ProtocolError> {
        let (key, _) = self.lookup(id)?;
        self.run_retrying("dht inspect", move |rc| {
            let (k, s) = (key.clone(), subkeys.as_ref().map(range));
            async move { rc.inspect_dht_record(k, s, scope).await }
        })
        .await
        .map_err(|e| to_protocol(&e))
    }

    async fn local_seq(&self, key: &RecordKey, subkey: u32) -> Result<Option<u32>, VeilidAPIError> {
        let k = key.clone();
        let report = self
            .run("dht local seq", move |rc| async move {
                rc.inspect_dht_record(
                    k,
                    Some(ValueSubkeyRangeSet::single(subkey)),
                    DHTReportScope::Local,
                )
                .await
            })
            .await?;
        Ok(report
            .local_seqs()
            .first()
            .and_then(veilid_core::ValueSeqNum::to_option))
    }

    /// Whether the local copy of `subkey` already holds `data` from the
    /// writer this write would sign with (the explicit one, else the
    /// lease's). Veilid does not advance the seq for such a write.
    async fn is_local_copy(
        &self,
        key: &RecordKey,
        subkey: u32,
        data: &[u8],
        writer: Option<&KeyPair>,
    ) -> Result<bool, VeilidAPIError> {
        let writer = match writer {
            Some(w) => Some(w.key()),
            None => self.table.lock().writer(&key.to_string()).map(KeyPair::key),
        };
        let Some(writer) = writer else {
            return Ok(false);
        };
        let k = key.clone();
        let local = self
            .run("dht local get", move |rc| async move {
                rc.get_dht_value(k, subkey, false).await
            })
            .await?;
        Ok(local.is_some_and(|v| v.data() == data && v.writer() == writer))
    }

    /// Write a subkey, with `writer` when it differs from the lease's.
    /// The size is checked before Veilid sees the value (R8).
    ///
    /// # Errors
    /// The lease is not held, the value is too large, or Veilid failed in a
    /// way that is not a network outcome.
    pub async fn set(
        &self,
        id: LeaseId,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<KeyPair>,
    ) -> Result<SetOutcome, ProtocolError> {
        let (key, opened) = self.lookup(id)?;
        if writer.is_none() && self.table.lock().writer(&key.to_string()).is_none() {
            return Err(ProtocolError::NotWritable(key.to_string()));
        }
        if data.len() > opened.key_cap {
            return Err(ProtocolError::SubkeyTooLarge {
                subkey,
                len: data.len(),
                cap: opened.key_cap,
            });
        }
        let before = self
            .local_seq(&key, subkey)
            .await
            .map_err(|e| to_protocol(&e))?;
        let identical = self
            .is_local_copy(&key, subkey, &data, writer.as_ref())
            .await
            .map_err(|e| to_protocol(&e))?;
        let k = key.clone();
        let result = self
            .run("dht set", move |rc| async move {
                rc.set_dht_value(
                    k,
                    subkey,
                    data,
                    Some(SetDHTValueOptions {
                        writer,
                        allow_offline: Some(AllowOffline(false)),
                    }),
                )
                .await
            })
            .await;
        match result {
            Ok(Some(newer)) => Ok(SetOutcome::Superseded(super::record_value(&newer))),
            Ok(None) => {
                let after = self
                    .local_seq(&key, subkey)
                    .await
                    .map_err(|e| to_protocol(&e))?;
                Ok(if identical {
                    // Same data and writer: Veilid kept the seq and re-pushed
                    // the copy (`set_value.rs:212-218`); `Ok(None)` is the
                    // same whether that reached consensus or not.
                    SetOutcome::Unchanged
                } else if after.is_some() && after != before {
                    SetOutcome::Landed
                } else {
                    SetOutcome::BelowConsensus
                })
            }
            Err(VeilidAPIError::TryAgain { .. }) => Ok(SetOutcome::Offline),
            Err(e) => Err(to_protocol(&e)),
        }
    }

    /// Re-open the leased record with its sticky writer: a re-open of a
    /// local record is what queues Veilid's rehydration
    /// (`open_record.rs:27-44`), so nothing is downgraded and the watch
    /// survives. The port's `rehydrate` then pulls what the network holds
    /// newer.
    ///
    /// # Errors
    /// The lease is not held, or the open failed within the retry budget.
    pub(super) async fn reopen_with_writer(&self, id: LeaseId) -> Result<(), ProtocolError> {
        let (key, _) = self.lookup(id)?;
        let name = key.to_string();
        let writer = self.table.lock().writer(&name).cloned();
        let lock = self.key_lock(&name);
        let _serial = lock.lock().await;
        self.run_retrying("dht rehydrate open", move |rc| {
            let (k, w) = (key.clone(), writer.clone());
            async move { rc.open_dht_record(k, w).await }
        })
        .await
        .map(drop)
        .map_err(|e| to_protocol(&e))
    }
}
