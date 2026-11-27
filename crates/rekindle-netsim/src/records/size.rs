//! Value sizes on the simulated DHT (plan E2.B slice S11, HIGH-1), after the
//! pinned veilid-core (`~/veilid-wt-058/veilid-core/src/storage_manager`). The
//! netsim keeps its own copy of Veilid's rule, so the product's cap
//! (`rekindle-records` `lease::max_subkey_bytes`) is tested against it:
//!
//! - a subkey's value is at most `min(MAX_SUBKEY_SIZE, MAX_RECORD_DATA_SIZE
//!   / subkey count)` bytes, the data's own length (`../veilid_api/types/
//!   dht/derivation.rs:141-148` for DFLT, whose subkey count is `o_cnt`,
//!   `:184-189` for SMPL; `dht/mod.rs:36,38`, `encrypted_value_data.rs:12`,
//!   `dht/value_data.rs:99-101`);
//! - the rule is checked at every hop: before a set leaves the node (a
//!   failure is `Generic` "failed schema validation", `set_value.rs:396-405`),
//!   at the replica (an invalid message, nothing stored, `set_value.rs:882`),
//!   on a get answer (the answer is invalid and skipped, `get_value.rs:462`)
//!   and on a watched value (the change is invalid and dropped,
//!   `watch_value.rs:2188-2200`);
//! - a record's subkeys together hold at most `MAX_RECORD_DATA_SIZE` bytes
//!   in a store: a store refusing it is `Internal` "record exceeds maximum
//!   data size" (`record_store/record.rs:105-129`). The per-subkey rule
//!   implies it; it is checked where Veilid checks it.

use rekindle_records::port::{RecordSchema, RecordValue};

/// `MAX_SUBKEY_SIZE` (`../veilid_api/types/dht/mod.rs:36`, `encrypted_value_data.rs:12`).
pub(crate) const MAX_SUBKEY_SIZE: usize = 32_768;
/// `MAX_RECORD_DATA_SIZE` (`../veilid_api/types/dht/mod.rs:38`).
pub(crate) const MAX_RECORD_DATA_SIZE: usize = 1_048_576;

/// The largest value one subkey of `schema` holds.
pub(crate) fn max_value_len(schema: &RecordSchema) -> usize {
    let count = schema.subkey_count().max(1);
    MAX_SUBKEY_SIZE.min(MAX_RECORD_DATA_SIZE / count)
}

/// Whether `value` passes the schema's size rule.
pub(crate) fn fits(schema: &RecordSchema, value: &RecordValue) -> bool {
    value.data.len() <= max_value_len(schema)
}

/// Whether a store holding `values` can take `value` at `subkey` within the
/// record's total (`record.rs:105-129`).
pub(crate) fn within_total(
    values: &[Option<RecordValue>],
    subkey: usize,
    value: &RecordValue,
) -> bool {
    let others: usize = values
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != subkey)
        .filter_map(|(_, v)| v.as_ref().map(|v| v.data.len()))
        .sum();
    others + value.data.len() <= MAX_RECORD_DATA_SIZE
}

/// Whether a replica holding `values` stores `value` at `subkey`: the
/// schema's rule, then the store's total.
pub(crate) fn admits(
    schema: &RecordSchema,
    values: &[Option<RecordValue>],
    subkey: usize,
    value: &RecordValue,
) -> bool {
    fits(schema, value) && within_total(values, subkey, value)
}

#[cfg(test)]
#[path = "size_tests.rs"]
mod tests;
