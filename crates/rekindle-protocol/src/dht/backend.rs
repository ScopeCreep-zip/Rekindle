//! `VeilidDht`: the record pool's [`DhtBackend`] over one Veilid
//! `RoutingContext` (plan E2.B, seam design 2026-10-21). The only code that
//! makes Veilid DHT record calls (`cargo xtask check-veilid-dht-calls`).
//!
//! It converts and nothing else: schemas, keys and keypairs from their text,
//! values, inspect reports and value changes to the port's types, and every
//! `VeilidAPIError` to a [`DhtCallError`] of the same kind with Veilid's own text.
//! What the errors mean (retry, offline, closed) is the pool's.

use rekindle_records::port::{
    DhtBackend, DhtCallError, DhtErrorKind, PortError, RecordDescriptor, RecordReport,
    RecordSchema, RecordValue, ReportScope, ValueChange, WriterKey,
};
use rekindle_types::config::{
    SafetyProfile, SequencingPreference, StabilityPreference, ANONYMITY_HOP_FLOOR,
};
use veilid_core::{
    AllowOffline, BareMemberId, DHTRecordDescriptor, DHTRecordReport, DHTReportScope, DHTSchema,
    DHTSchemaSMPLMember, KeyPair, RecordKey, RoutingContext, SafetySelection, SafetySpec,
    Sequencing, SetDHTValueOptions, Stability, Timestamp, ValueData, ValueSeqNum,
    ValueSubkeyRangeSet, VeilidAPIError, VeilidValueChange, CRYPTO_KIND_VLD0,
};

use crate::ProtocolError;

/// The one mapping from a data class's profile to a Veilid safety
/// selection (V19). Always Safe, never below [`ANONYMITY_HOP_FLOOR`].
pub fn safety_selection(profile: &SafetyProfile) -> SafetySelection {
    SafetySelection::Safe(SafetySpec {
        preferred_route: None,
        hop_count: usize::from(profile.hop_count.max(ANONYMITY_HOP_FLOOR)),
        stability: match profile.stability {
            StabilityPreference::LowLatency => Stability::LowLatency,
            StabilityPreference::Reliable => Stability::Reliable,
        },
        sequencing: match profile.sequencing {
            SequencingPreference::NoPreference => Sequencing::PreferUnordered,
            SequencingPreference::PreferOrdered => Sequencing::PreferOrdered,
            SequencingPreference::EnsureOrdered => Sequencing::EnsureOrdered,
        },
    })
}

/// A Veilid error as the backend reports it: the same kind, Veilid's text.
#[must_use]
pub fn dht_error(e: &VeilidAPIError) -> DhtCallError {
    let kind = match e {
        VeilidAPIError::NotInitialized => DhtErrorKind::NotInitialized,
        VeilidAPIError::AlreadyInitialized => DhtErrorKind::AlreadyInitialized,
        VeilidAPIError::Timeout => DhtErrorKind::Timeout,
        VeilidAPIError::TryAgain { .. } => DhtErrorKind::TryAgain,
        VeilidAPIError::InvalidTarget { .. } => DhtErrorKind::InvalidTarget,
        VeilidAPIError::NoConnection { .. } => DhtErrorKind::NoConnection,
        VeilidAPIError::Shutdown => DhtErrorKind::Shutdown,
        VeilidAPIError::KeyNotFound { .. } => DhtErrorKind::KeyNotFound,
        VeilidAPIError::Internal { .. } => DhtErrorKind::Internal,
        VeilidAPIError::Unimplemented { .. } => DhtErrorKind::Unimplemented,
        VeilidAPIError::ParseError { .. } => DhtErrorKind::ParseError,
        VeilidAPIError::InvalidArgument { .. } => DhtErrorKind::InvalidArgument,
        VeilidAPIError::MissingArgument { .. } => DhtErrorKind::MissingArgument,
        VeilidAPIError::Generic { .. } => DhtErrorKind::Generic,
        VeilidAPIError::TransactionNotFound { .. } => DhtErrorKind::TransactionNotFound,
    };
    DhtCallError {
        kind,
        message: e.to_string(),
    }
}

fn parse_error(message: String) -> DhtCallError {
    DhtCallError {
        kind: DhtErrorKind::ParseError,
        message,
    }
}

/// A keypair from its text.
///
/// # Errors
/// The text is not a keypair.
pub fn parse_keypair(text: &str) -> Result<KeyPair, PortError> {
    text.parse::<KeyPair>()
        .map_err(|e| PortError::KeyText(format!("keypair: {e}")))
}

fn keypair(writer: &WriterKey) -> Result<KeyPair, DhtCallError> {
    parse_keypair(&writer.keypair).map_err(|e| parse_error(e.to_string()))
}

fn record_key(key: &str) -> Result<RecordKey, DhtCallError> {
    crate::dht::parse_record_key(key).map_err(|e| parse_error(e.to_string()))
}

fn writer_key(keypair: &KeyPair) -> WriterKey {
    WriterKey {
        keypair: keypair.to_string(),
        public: keypair.key().to_string(),
    }
}

/// The Veilid schema for `schema`.
///
/// # Errors
/// Veilid refuses it (no subkeys, or too many).
pub fn dht_schema(schema: &RecordSchema) -> Result<DHTSchema, PortError> {
    veilid_schema(schema).map_err(|e| PortError::Dht(e.message))
}

fn veilid_schema(schema: &RecordSchema) -> Result<DHTSchema, DhtCallError> {
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
    built.map_err(|e| DhtCallError {
        message: format!("schema: {e}"),
        ..dht_error(&e)
    })
}

fn descriptor(descriptor: &DHTRecordDescriptor) -> RecordDescriptor {
    RecordDescriptor {
        key: descriptor.key().to_string(),
        // As Veilid builds it from the descriptor's owner and secret.
        owner: descriptor
            .owner_secret()
            .map(|secret| writer_key(&KeyPair::new_from_parts(descriptor.owner(), secret.value()))),
        owner_public: descriptor.owner().to_string(),
        subkey_count: u32::try_from(descriptor.schema().subkey_count()).unwrap_or(u32::MAX),
    }
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

fn range(subkeys: Option<&[u32]>) -> Option<ValueSubkeyRangeSet> {
    subkeys.map(|s| s.iter().copied().collect())
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

/// A Veilid value as the port carries it.
#[must_use]
pub fn record_value(value: &ValueData) -> RecordValue {
    RecordValue {
        seq: u32::from(value.seq()),
        data: value.data().to_vec(),
        writer: value.writer().to_string(),
    }
}

/// A Veilid value change as the port carries it.
#[must_use]
pub fn value_change(change: &VeilidValueChange) -> ValueChange {
    ValueChange {
        key: change.key.to_string(),
        subkeys: change.subkeys.iter().collect(),
        count: change.count,
        value: change.value.as_ref().map(record_value),
    }
}

/// The pool's DHT backend: one routing context, its safety selection pinned.
pub struct VeilidDht {
    rc: RoutingContext,
}

impl VeilidDht {
    /// A backend over `rc` with the safety selection pinned from `profile`,
    /// so no record is ever re-opened at Veilid's 1-hop default (V19).
    ///
    /// # Errors
    /// The routing context cannot take the safety selection.
    pub fn new(rc: &RoutingContext, profile: &SafetyProfile) -> Result<Self, ProtocolError> {
        let rc = rc
            .clone()
            .with_safety(safety_selection(profile))
            .map_err(|e| ProtocolError::RoutingError(format!("pool safety selection: {e}")))?;
        Ok(Self { rc })
    }
}

#[async_trait::async_trait]
impl DhtBackend for VeilidDht {
    fn parse_writer(&self, text: &str) -> Result<WriterKey, DhtCallError> {
        let parsed = parse_keypair(text).map_err(|e| match e {
            PortError::KeyText(m) => parse_error(m),
            other => parse_error(other.to_string()),
        })?;
        Ok(writer_key(&parsed))
    }

    fn check_key(&self, key: &str) -> Result<String, DhtCallError> {
        Ok(record_key(key)?.to_string())
    }

    async fn create(
        &self,
        schema: &RecordSchema,
        owner: Option<&WriterKey>,
    ) -> Result<RecordDescriptor, DhtCallError> {
        let schema = veilid_schema(schema)?;
        let owner = owner.map(keypair).transpose()?;
        self.rc
            .create_dht_record(CRYPTO_KIND_VLD0, schema, owner)
            .await
            .map(|d| descriptor(&d))
            .map_err(|e| dht_error(&e))
    }

    async fn open(
        &self,
        key: &str,
        writer: Option<&WriterKey>,
    ) -> Result<RecordDescriptor, DhtCallError> {
        let writer = writer.map(keypair).transpose()?;
        self.rc
            .open_dht_record(record_key(key)?, writer)
            .await
            .map(|d| descriptor(&d))
            .map_err(|e| dht_error(&e))
    }

    async fn close(&self, key: &str) -> Result<(), DhtCallError> {
        self.rc
            .close_dht_record(record_key(key)?)
            .await
            .map_err(|e| dht_error(&e))
    }

    async fn get(
        &self,
        key: &str,
        subkey: u32,
        force_refresh: bool,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        self.rc
            .get_dht_value(record_key(key)?, subkey, force_refresh)
            .await
            .map(|v| v.as_ref().map(record_value))
            .map_err(|e| dht_error(&e))
    }

    async fn set(
        &self,
        key: &str,
        subkey: u32,
        data: Vec<u8>,
        writer: Option<&WriterKey>,
    ) -> Result<Option<RecordValue>, DhtCallError> {
        let writer = writer.map(keypair).transpose()?;
        self.rc
            .set_dht_value(
                record_key(key)?,
                subkey,
                data,
                Some(SetDHTValueOptions {
                    writer,
                    allow_offline: Some(AllowOffline(false)),
                }),
            )
            .await
            .map(|v| v.as_ref().map(record_value))
            .map_err(|e| dht_error(&e))
    }

    async fn inspect(
        &self,
        key: &str,
        subkeys: Option<&[u32]>,
        report_scope: ReportScope,
    ) -> Result<RecordReport, DhtCallError> {
        self.rc
            .inspect_dht_record(record_key(key)?, range(subkeys), scope(report_scope))
            .await
            .map(|r| record_report(&r))
            .map_err(|e| dht_error(&e))
    }

    async fn watch(
        &self,
        key: &str,
        subkeys: Option<&[u32]>,
        expiration_us: Option<u64>,
        count: Option<u32>,
    ) -> Result<bool, DhtCallError> {
        self.rc
            .watch_dht_values(
                record_key(key)?,
                range(subkeys),
                expiration_us.map(Timestamp::new),
                count,
            )
            .await
            .map_err(|e| dht_error(&e))
    }

    async fn cancel_watch(&self, key: &str, subkeys: Option<&[u32]>) -> Result<bool, DhtCallError> {
        self.rc
            .cancel_dht_watch(record_key(key)?, range(subkeys))
            .await
            .map_err(|e| dht_error(&e))
    }
}

#[cfg(test)]
mod tests;
