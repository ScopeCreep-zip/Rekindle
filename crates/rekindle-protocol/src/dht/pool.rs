//! The session's record pool (plan C7.2, D4) over Veilid. The pool itself
//! is `rekindle-record-pool` (ADR 0021); this crate keeps its Veilid backend
//! ([`crate::dht::backend::VeilidDht`]) and the Veilid conversions.

pub use crate::dht::backend::{
    dht_schema, parse_keypair, record_report, safety_selection, value_change,
};
pub use rekindle_record_pool::{
    as_port, keepalive, set_failed, to_port, RecordCloser, RecordPool, CAS_ROUNDS, RETRY_BUDGET,
};
