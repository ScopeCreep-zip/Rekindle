//! Reads, writes and inspects on leased records.

use rekindle_records::lease::{LeaseId, SubkeySet};
use rekindle_records::port::{
    DhtCallError, DhtErrorKind, RecordReport, RecordValue, ReportScope, SetOutcome, WriterKey,
};

use super::{range, to_port, RecordPool};
use rekindle_records::port::PortError;

/// How the pool reports a set that failed: `TryAgain` is a write short of consensus, the node
/// offline or the fanout below consensus, which Veilid answers alike (`set_value.rs:710-738`);
/// nothing was stored locally or queued. Anything else is the error.
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

    /// The public key a write would sign with: the explicit writer, else
    /// the lease's.
    fn signer(&self, key: &str, writer: Option<&WriterKey>) -> Option<String> {
        match writer {
            Some(w) => Some(w.public.clone()),
            None => self.table.lock().writer(key).map(|w| w.public.clone()),
        }
    }

    /// Whether the local copy of `subkey` already holds `data` from
    /// `signer`. Veilid does not advance the seq for such a write.
    async fn is_local_copy(
        &self,
        key: &str,
        subkey: u32,
        data: &[u8],
        signer: Option<&str>,
    ) -> Result<bool, DhtCallError> {
        let Some(signer) = signer else {
            return Ok(false);
        };
        let k = key.to_string();
        let local = self
            .run("dht local get", move |backend| async move {
                backend.get(&k, subkey, false).await
            })
            .await?;
        Ok(local.is_some_and(|v| v.data == data && v.writer == signer))
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
        let signer = self.signer(&key, writer.as_ref());
        // Only a subkey with a local copy is compared: veilid-core's get
        // with no local value goes to the network even without
        // `force_refresh` (`get_value.rs:72-92`), a full fanout online and
        // `TryAgain` offline.
        let identical = before.is_some()
            && self
                .is_local_copy(&key, subkey, &data, signer.as_deref())
                .await
                .map_err(|e| to_port(&e))?;
        let k = key.clone();
        let sent = data.clone();
        let result = self
            .run("dht set", move |backend| async move {
                backend.set(&k, subkey, data, writer.as_ref()).await
            })
            .await;
        match result {
            // Veilid 0.5.8 returns the value the set settled on whenever it
            // differs from ours *or* some node holds a rival at its seq
            // (`set_value.rs:753-760`). Ours settling is a stored write; the
            // rival is only logged (plan VU decision 4).
            Ok(Some(settled))
                if signer.as_deref() == Some(settled.writer.as_str()) && settled.data == sent =>
            {
                tracing::debug!(
                    key,
                    subkey,
                    seq = settled.seq,
                    "dht set: our value settled; the subkey is contested"
                );
                let after = self
                    .local_seq(&key, subkey)
                    .await
                    .map_err(|e| to_port(&e))?;
                Ok(if after.is_some() && after != before {
                    SetOutcome::Landed
                } else {
                    SetOutcome::Unchanged
                })
            }
            Ok(Some(rival)) => Ok(SetOutcome::Superseded(rival)),
            // `None` is ours at consensus: below it Veilid answers
            // `TryAgain` and stores nothing (`set_value.rs:731-738` →
            // `local_record_store_interface.rs:60-62`), mapped by
            // `set_failed`. An identical value kept its seq and re-sent the
            // stored bytes (`set_value.rs:353-366`).
            Ok(None) => Ok(if identical {
                SetOutcome::Unchanged
            } else {
                SetOutcome::Landed
            }),
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
