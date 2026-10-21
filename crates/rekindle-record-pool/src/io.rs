//! Reads, writes and inspects on leased records.

use rekindle_records::lease::{LeaseId, SubkeySet};
use rekindle_records::port::{
    DhtCallError, DhtErrorKind, RecordReport, RecordValue, ReportScope, SetOutcome, WriterKey,
};

use super::{range, to_port, RecordPool};
use rekindle_records::port::PortError;

/// How the pool reports a set that failed: `TryAgain` is the node being offline (nothing stored
/// or queued), anything else is the error.
///
/// # Errors
/// Every kind but `TryAgain`, as [`to_port`](crate::to_port).
pub fn set_failed(e: &DhtCallError) -> Result<SetOutcome, PortError> {
    if e.kind == DhtErrorKind::TryAgain {
        Ok(SetOutcome::Offline)
    } else {
        Err(to_port(e))
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
    ) -> Result<Option<RecordValue>, PortError> {
        let (key, _) = self.lookup(id)?;
        self.run_retrying("dht get", move |backend| {
            let k = key.clone();
            async move { backend.get(&k, subkey, force_refresh).await }
        })
        .await
        .map_err(|e| to_port(&e))
    }

    /// Inspect the leased record.
    ///
    /// # Errors
    /// The lease is not held, or the inspect failed within the retry budget.
    pub async fn inspect(
        &self,
        id: LeaseId,
        subkeys: Option<SubkeySet>,
        scope: ReportScope,
    ) -> Result<RecordReport, PortError> {
        let (key, _) = self.lookup(id)?;
        self.run_retrying("dht inspect", move |backend| {
            let (k, s) = (key.clone(), subkeys.as_ref().map(range));
            async move { backend.inspect(&k, s.as_deref(), scope).await }
        })
        .await
        .map_err(|e| to_port(&e))
    }

    async fn local_seq(&self, key: &str, subkey: u32) -> Result<Option<u32>, DhtCallError> {
        let k = key.to_string();
        let report = self
            .run("dht local seq", move |backend| async move {
                backend
                    .inspect(&k, Some(&[subkey]), ReportScope::Local)
                    .await
            })
            .await?;
        Ok(report.local_seqs.first().copied().flatten())
    }

    /// Whether the local copy of `subkey` already holds `data` from the
    /// writer this write would sign with (the explicit one, else the
    /// lease's). Veilid does not advance the seq for such a write.
    async fn is_local_copy(
        &self,
        key: &str,
        subkey: u32,
        data: &[u8],
        writer: Option<&WriterKey>,
    ) -> Result<bool, DhtCallError> {
        let writer = match writer {
            Some(w) => Some(w.public.clone()),
            None => self.table.lock().writer(key).map(|w| w.public.clone()),
        };
        let Some(writer) = writer else {
            return Ok(false);
        };
        let k = key.to_string();
        let local = self
            .run("dht local get", move |backend| async move {
                backend.get(&k, subkey, false).await
            })
            .await?;
        Ok(local.is_some_and(|v| v.data == data && v.writer == writer))
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
        writer: Option<WriterKey>,
    ) -> Result<SetOutcome, PortError> {
        let (key, opened) = self.lookup(id)?;
        if writer.is_none() && self.table.lock().writer(&key).is_none() {
            return Err(PortError::NotWritable(key));
        }
        if data.len() > opened.key_cap {
            return Err(PortError::SubkeyTooLarge {
                subkey,
                len: data.len(),
                cap: opened.key_cap,
            });
        }
        let before = self
            .local_seq(&key, subkey)
            .await
            .map_err(|e| to_port(&e))?;
        let identical = self
            .is_local_copy(&key, subkey, &data, writer.as_ref())
            .await
            .map_err(|e| to_port(&e))?;
        let k = key.clone();
        let result = self
            .run("dht set", move |backend| async move {
                backend.set(&k, subkey, data, writer.as_ref()).await
            })
            .await;
        match result {
            Ok(Some(newer)) => Ok(SetOutcome::Superseded(newer)),
            Ok(None) => {
                let after = self
                    .local_seq(&key, subkey)
                    .await
                    .map_err(|e| to_port(&e))?;
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
            Err(e) => set_failed(&e),
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
    pub(super) async fn reopen_with_writer(&self, id: LeaseId) -> Result<(), PortError> {
        let (key, _) = self.lookup(id)?;
        let writer = self.table.lock().writer(&key).cloned();
        let lock = self.key_lock(&key);
        let _serial = lock.lock().await;
        self.run_retrying("dht rehydrate open", move |backend| {
            let (k, w) = (key.clone(), writer.clone());
            async move { backend.open(&k, w.as_ref()).await }
        })
        .await
        .map(drop)
        .map_err(|e| to_port(&e))
    }
}
