//! [`RecordPool`] as the Veilid-free [`RecordPort`] (plan C9.1): keys and
//! keypairs as their text (checked by the pool's backend), values as
//! [`RecordValue`], schemas as [`RecordSchema`].

use std::sync::Arc;

use rekindle_records::lease::LeaseId;
use rekindle_records::port::{
    PortError, RecordPort, RecordReport, RecordSchema, RecordValue, ReportScope, SetOutcome,
    Settled, UnsentWrite, ValueChange, ValueChangeReport, WriterKey,
};

use super::RecordPool;

impl RecordPool {
    fn writer(&self, text: Option<&str>) -> Result<Option<WriterKey>, PortError> {
        text.map(|t| self.backend.parse_writer(t))
            .transpose()
            .map_err(|e| PortError::KeyText(e.message))
    }
}

#[async_trait::async_trait]
impl RecordPort for RecordPool {
    async fn acquire(&self, key: &str, writer_text: Option<&str>) -> Result<LeaseId, PortError> {
        let key = self
            .backend
            .check_key(key)
            .map_err(|e| PortError::KeyText(e.message))?;
        Ok(RecordPool::acquire(self, &key, self.writer(writer_text)?).await?)
    }

    async fn create(
        &self,
        schema: &RecordSchema,
        owner: Option<&str>,
    ) -> Result<(LeaseId, String, String), PortError> {
        let (lease, key, keypair) =
            RecordPool::create(self, schema.clone(), self.writer(owner)?).await?;
        Ok((lease, key, keypair.keypair))
    }

    fn key_of(&self, lease: LeaseId) -> Option<String> {
        RecordPool::key_of(self, lease)
    }

    async fn release(&self, lease: LeaseId) {
        RecordPool::release(self, lease).await;
    }

    async fn get(
        &self,
        lease: LeaseId,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<RecordValue>, PortError> {
        Ok(RecordPool::get(self, lease, subkey, force_refresh).await?)
    }

    async fn set(
        &self,
        lease: LeaseId,
        subkey: u32,
        data: Vec<u8>,
        writer_text: Option<&str>,
    ) -> Result<SetOutcome, PortError> {
        Ok(RecordPool::set(self, lease, subkey, data, self.writer(writer_text)?).await?)
    }

    async fn set_durable(
        &self,
        lease: LeaseId,
        subkey: u32,
        data: Vec<u8>,
        writer_text: Option<&str>,
    ) -> Result<SetOutcome, PortError> {
        let writer = self.writer(writer_text)?;
        Ok(RecordPool::set_durable_as(self, lease, subkey, data, writer).await?)
    }

    async fn inspect(
        &self,
        lease: LeaseId,
        subkeys: Option<&[u32]>,
        report_scope: ReportScope,
    ) -> Result<RecordReport, PortError> {
        let set = subkeys.map(|s| s.iter().copied().collect());
        Ok(RecordPool::inspect(self, lease, set, report_scope).await?)
    }

    async fn read_changed(
        &self,
        lease: LeaseId,
        subkeys: &[u32],
    ) -> Result<Vec<(u32, RecordValue)>, PortError> {
        Ok(RecordPool::read_changed(self, lease, subkeys).await?)
    }

    async fn rehydrate(&self, lease: LeaseId) -> Result<(), PortError> {
        RecordPool::reopen_with_writer(self, lease).await?;
        for subkey in self.inspect_present(lease).await? {
            RecordPort::get(self, lease, subkey, true).await?;
        }
        Ok(())
    }

    async fn watch(&self, lease: LeaseId, subkeys: &[u32]) -> Result<(), PortError> {
        Ok(RecordPool::watch(self, lease, subkeys.iter().copied().collect()).await?)
    }

    async fn watch_all(&self, lease: LeaseId) -> Result<(), PortError> {
        Ok(RecordPool::watch_all(self, lease).await?)
    }

    fn subkey_cap(&self, lease: LeaseId) -> Result<usize, PortError> {
        Ok(RecordPool::subkey_cap(self, lease)?)
    }

    fn on_value_change(&self, change: &ValueChange) -> ValueChangeReport {
        match self.this.upgrade() {
            Some(pool) => RecordPool::on_value_change(&pool, change),
            // The pool is being dropped: nothing is held.
            None => ValueChangeReport {
                held: false,
                watch_died: change.count == 0 || change.subkeys.is_empty(),
                subkeys: change.subkeys.clone(),
                value: change.value.clone(),
            },
        }
    }

    fn settled(&self) -> tokio::sync::broadcast::Receiver<Settled> {
        RecordPool::settled(self)
    }

    fn held_writes(&self) -> Vec<(String, u32)> {
        RecordPool::held_writes(self)
    }

    fn take_held(&self) -> Vec<UnsentWrite> {
        RecordPool::take_held(self)
    }

    fn restore_held(&self, writes: Vec<UnsentWrite>) {
        RecordPool::restore_held(self, writes);
    }

    fn drain(&self) {
        RecordPool::drain(self);
    }

    fn admit_teardown(&self) {
        RecordPool::admit_teardown(self);
    }

    fn end(&self) -> usize {
        let in_flight = self.scope().len();
        let calls = self.scope().running();
        let records = RecordPool::end(self);
        tracing::info!(in_flight, ?calls, records, "record pool ended");
        records
    }
}

/// The pool as a port, for the transport node.
#[must_use]
pub fn as_port(pool: Arc<RecordPool>) -> Arc<dyn RecordPort> {
    pool
}
