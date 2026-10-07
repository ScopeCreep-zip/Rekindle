//! Reads, writes and inspects on leased records.

use rekindle_records::lease::{LeaseId, SubkeySet};
use veilid_core::{
    AllowOffline, DHTRecordReport, DHTReportScope, KeyPair, RecordKey, SetDHTValueOptions,
    ValueData, ValueSubkeyRangeSet, VeilidAPIError,
};

use super::{range, to_protocol, RecordPool};
use crate::ProtocolError;

/// What a write did on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetOutcome {
    /// Stored at consensus.
    Landed,
    /// The fanout finished below consensus; nothing was stored or queued.
    BelowConsensus,
    /// The node is offline; nothing was stored or queued.
    Offline,
    /// The network already holds a newer value, returned here.
    Superseded(ValueData),
    /// The value equals the local copy (same data and writer), so Veilid
    /// kept its seq and re-pushed it, and no newer value came back. Veilid
    /// answers this the same whether the re-push reached consensus or not
    /// (`set_value.rs:212-218`, `:170`), so the write itself proves nothing
    /// about the network copy; `rehydrate` owns re-pushing a record.
    Unchanged,
}

impl SetOutcome {
    /// `Ok` unless the write [`missed`](Self::missed): for a write that a
    /// structure's next step depends on (a head, a spine, a slot).
    ///
    /// # Errors
    /// [`ProtocolError::NotStored`] for a missed write.
    pub fn require_stored(self, subkey: u32) -> Result<(), ProtocolError> {
        if self.missed() {
            return Err(ProtocolError::NotStored {
                subkey,
                outcome: match self {
                    Self::Superseded(_) => "superseded by a newer value".into(),
                    other => format!("{other:?}"),
                },
            });
        }
        Ok(())
    }

    /// The write did not reach consensus, or lost to a newer value. An
    /// `Unchanged` re-push is not a miss: it re-sent a value already held
    /// locally, an idempotent republish, whose network copy is
    /// `rehydrate`'s to keep alive.
    #[must_use]
    pub fn missed(&self) -> bool {
        matches!(
            self,
            Self::BelowConsensus | Self::Offline | Self::Superseded(_)
        )
    }
}

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

    /// Subkeys the network holds newer than the local copy
    /// (`UpdateGet` inspect).
    ///
    /// # Errors
    /// As [`inspect`](Self::inspect).
    pub async fn inspect_present(&self, id: LeaseId) -> Result<Vec<u32>, ProtocolError> {
        let report = self.inspect(id, None, DHTReportScope::UpdateGet).await?;
        Ok(newer_on_network(&report))
    }

    /// Subkeys written while offline that Veilid has not yet flushed: show
    /// them as pending (R6).
    ///
    /// # Errors
    /// As [`inspect`](Self::inspect).
    pub async fn pending_writes(&self, id: LeaseId) -> Result<Vec<u32>, ProtocolError> {
        let report = self.inspect(id, None, DHTReportScope::Local).await?;
        Ok(report.offline_subkeys().iter().collect())
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
            Ok(Some(newer)) => Ok(SetOutcome::Superseded(newer)),
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

    /// Re-push the leased record to the network and pull anything newer.
    /// A re-open of a local record is what queues Veilid's rehydration
    /// (`open_record.rs:27-44`); it re-opens with the sticky writer, so
    /// nothing is downgraded and the watch survives.
    ///
    /// # Errors
    /// The lease is not held, or a step failed within the retry budget.
    pub async fn rehydrate(&self, id: LeaseId) -> Result<(), ProtocolError> {
        let (key, _) = self.lookup(id)?;
        let name = key.to_string();
        let writer = self.table.lock().writer(&name).cloned();
        {
            let lock = self.key_lock(&name);
            let _serial = lock.lock().await;
            let k = key.clone();
            self.run_retrying("dht rehydrate open", move |rc| {
                let (k, w) = (k.clone(), writer.clone());
                async move { rc.open_dht_record(k, w).await }
            })
            .await
            .map(drop)
            .map_err(|e| to_protocol(&e))?;
        }
        for subkey in self.inspect_present(id).await? {
            self.get(id, subkey, true).await?;
        }
        Ok(())
    }
}

/// Subkeys whose network seq is ahead of the local one in `report`.
fn newer_on_network(report: &DHTRecordReport) -> Vec<u32> {
    report
        .subkeys()
        .iter()
        .zip(report.local_seqs().iter().zip(report.network_seqs()))
        .filter(|(_, (local, network))| network.to_option() > local.to_option())
        .map(|(subkey, _)| subkey)
        .collect()
}

impl RecordPool {
    /// Read one subkey of `key` on a borrow of its own (acquire, get,
    /// release). While the session holds the record the borrow is a table
    /// hit with no Veilid call.
    ///
    /// # Errors
    /// The record could not be opened or read.
    pub async fn read_once(
        &self,
        key: &RecordKey,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<ValueData>, ProtocolError> {
        let lease = self.acquire(key, None).await?;
        let value = self.get(lease, subkey, force_refresh).await;
        self.release(lease).await;
        value
    }

    /// Write one subkey of `key` as `writer` on a borrow of its own.
    ///
    /// # Errors
    /// The record could not be opened, or the write failed outright.
    pub async fn write_once(
        &self,
        key: &RecordKey,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<KeyPair>,
    ) -> Result<SetOutcome, ProtocolError> {
        let lease = self.acquire(key, None).await?;
        let outcome = self.set(lease, subkey, data, writer).await;
        self.release(lease).await;
        outcome
    }

    /// Inspect `key` on a borrow of its own.
    ///
    /// # Errors
    /// The record could not be opened or inspected.
    pub async fn inspect_once(
        &self,
        key: &RecordKey,
        subkeys: Option<SubkeySet>,
        scope: DHTReportScope,
    ) -> Result<DHTRecordReport, ProtocolError> {
        let lease = self.acquire(key, None).await?;
        let report = self.inspect(lease, subkeys, scope).await;
        self.release(lease).await;
        report
    }
}
