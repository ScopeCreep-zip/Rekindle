//! [`RecordPool`] as the Veilid-free [`RecordPort`] (plan C9.1): keys and
//! keypairs as their text, values as [`RecordValue`], schemas as
//! [`RecordSchema`].

use std::sync::Arc;

use rekindle_records::lease::LeaseId;
use rekindle_records::port::{
    PortError, RecordPort, RecordReport, RecordSchema, RecordValue, ReportScope, SetOutcome,
    Settled, UnsentWrite, ValueChange, ValueChangeReport,
};
use veilid_core::{
    BareMemberId, DHTRecordReport, DHTReportScope, DHTSchema, DHTSchemaSMPLMember, KeyPair,
    ValueSeqNum,
};

use super::RecordPool;
use crate::dht::parse_record_key;

/// A keypair from its text.
///
/// # Errors
/// The text is not a keypair.
pub fn parse_keypair(text: &str) -> Result<KeyPair, PortError> {
    text.parse::<KeyPair>()
        .map_err(|e| PortError::KeyText(format!("keypair: {e}")))
}

fn writer(text: Option<&str>) -> Result<Option<KeyPair>, PortError> {
    text.map(parse_keypair).transpose()
}

/// The Veilid schema for `schema`.
///
/// # Errors
/// Veilid refuses it (no subkeys, or too many).
pub fn dht_schema(schema: &RecordSchema) -> Result<DHTSchema, PortError> {
    let built = match schema {
        RecordSchema::Dflt { owner_subkeys } => DHTSchema::dflt(*owner_subkeys),
        RecordSchema::Smpl {
            owner_subkeys,
            members,
        } => DHTSchema::smpl(
            *owner_subkeys,
            members
                .iter()
                .map(|m| DHTSchemaSMPLMember {
                    m_key: BareMemberId::new(&m.key),
                    m_cnt: m.subkeys,
                })
                .collect(),
        ),
    };
    built.map_err(|e| PortError::Dht(format!("schema: {e}")))
}

fn scope(scope: ReportScope) -> DHTReportScope {
    match scope {
        ReportScope::Local => DHTReportScope::Local,
        ReportScope::SyncGet => DHTReportScope::SyncGet,
        ReportScope::UpdateGet => DHTReportScope::UpdateGet,
        ReportScope::SyncSet => DHTReportScope::SyncSet,
        ReportScope::UpdateSet => DHTReportScope::UpdateSet,
    }
}

/// A Veilid inspect report as the port carries it.
#[must_use]
pub fn record_report(report: &DHTRecordReport) -> RecordReport {
    RecordReport {
        subkeys: report.subkeys().iter().collect(),
        local_seqs: report
            .local_seqs()
            .iter()
            .map(ValueSeqNum::to_option)
            .collect(),
        network_seqs: report
            .network_seqs()
            .iter()
            .map(ValueSeqNum::to_option)
            .collect(),
        offline_subkeys: report.offline_subkeys().iter().collect(),
    }
}

#[async_trait::async_trait]
impl RecordPort for RecordPool {
    async fn acquire(&self, key: &str, writer_text: Option<&str>) -> Result<LeaseId, PortError> {
        let key = parse_record_key(key).map_err(|e| PortError::KeyText(e.to_string()))?;
        Ok(RecordPool::acquire(self, &key, writer(writer_text)?).await?)
    }

    async fn create(
        &self,
        schema: &RecordSchema,
        owner: Option<&str>,
    ) -> Result<(LeaseId, String, String), PortError> {
        let (lease, key, keypair) =
            RecordPool::create(self, dht_schema(schema)?, writer(owner)?).await?;
        Ok((lease, key.to_string(), keypair.to_string()))
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
        Ok(RecordPool::get(self, lease, subkey, force_refresh)
            .await?
            .as_ref()
            .map(super::record_value))
    }

    async fn set(
        &self,
        lease: LeaseId,
        subkey: u32,
        data: Vec<u8>,
        writer_text: Option<&str>,
    ) -> Result<SetOutcome, PortError> {
        Ok(RecordPool::set(self, lease, subkey, data, writer(writer_text)?).await?)
    }

    async fn set_durable(
        &self,
        lease: LeaseId,
        subkey: u32,
        data: Vec<u8>,
        writer_text: Option<&str>,
    ) -> Result<SetOutcome, PortError> {
        Ok(RecordPool::set_durable_as(self, lease, subkey, data, writer(writer_text)?).await?)
    }

    async fn inspect(
        &self,
        lease: LeaseId,
        subkeys: Option<&[u32]>,
        report_scope: ReportScope,
    ) -> Result<RecordReport, PortError> {
        let set = subkeys.map(|s| s.iter().copied().collect());
        let report = RecordPool::inspect(self, lease, set, scope(report_scope)).await?;
        Ok(record_report(&report))
    }

    async fn read_changed(
        &self,
        lease: LeaseId,
        subkeys: &[u32],
    ) -> Result<Vec<(u32, RecordValue)>, PortError> {
        Ok(RecordPool::read_changed(self, lease, subkeys)
            .await?
            .iter()
            .map(|(subkey, value)| (*subkey, super::record_value(value)))
            .collect())
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

#[cfg(test)]
mod tests {
    use rekindle_records::port::{RecordSchema, SchemaMember};
    use veilid_core::{
        BarePublicKey, BareSecretKey, KeyPair, PublicKey, ValueSeqNum, CRYPTO_KIND_VLD0,
    };

    use super::{dht_schema, parse_keypair};

    /// The Veilid-free keypair text (`rekindle_crypto::keypair_text`, plan
    /// C9.1b) is exactly what Veilid prints and parses.
    #[test]
    fn keypair_text_is_veilids() {
        let signing = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let public = signing.verifying_key().to_bytes();
        let secret = signing.to_bytes();
        let veilid = KeyPair::new_from_parts(
            PublicKey::new(CRYPTO_KIND_VLD0, BarePublicKey::new(&public)),
            BareSecretKey::new(&secret),
        );
        let text = rekindle_crypto::keypair_text::keypair_text(&public, &secret);
        assert_eq!(veilid.to_string(), text);
        assert_eq!(parse_keypair(&text).unwrap(), veilid);
        assert_eq!(
            rekindle_crypto::keypair_text::keypair_text_from_signing(&signing),
            text
        );
    }

    /// The fact every occupancy check rests on: Veilid's first write to a
    /// subkey lands at seq 0, so `Some(0)` is occupied, not empty
    /// (`record_report` maps through `to_option`). Two readers once
    /// flattened `None` to 0 and tested `!= 0`, classifying every write-once
    /// member as absent.
    #[test]
    fn seq_zero_is_written_not_empty() {
        let seqs = [
            ValueSeqNum::NONE,
            ValueSeqNum::from(0),
            ValueSeqNum::from(1),
            ValueSeqNum::NONE,
        ];
        let mapped: Vec<Option<u32>> = seqs.iter().map(ValueSeqNum::to_option).collect();
        assert_eq!(mapped, vec![None, Some(0), Some(1), None]);
    }

    /// The port's schema and Veilid's agree on every subkey count.
    #[test]
    fn schemas_keep_their_subkey_counts() {
        let dflt = RecordSchema::Dflt { owner_subkeys: 9 };
        assert_eq!(
            dht_schema(&dflt).unwrap().max_subkey() as usize + 1,
            dflt.subkey_count()
        );
        let smpl = RecordSchema::Smpl {
            owner_subkeys: 0,
            members: (1u8..=3)
                .map(|b| SchemaMember {
                    key: [b; 32],
                    subkeys: 1,
                })
                .collect(),
        };
        assert_eq!(
            dht_schema(&smpl).unwrap().max_subkey() as usize + 1,
            smpl.subkey_count()
        );
        assert!(dht_schema(&RecordSchema::Dflt { owner_subkeys: 0 }).is_err());
    }
}
