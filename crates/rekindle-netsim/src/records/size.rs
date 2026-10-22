//! Value sizes on the simulated DHT (plan E2.B slice S11, HIGH-1), after the
//! pinned veilid-core (`~/veilid/veilid-core/src/storage_manager`). The
//! netsim keeps its own copy of Veilid's rule, so the product's cap
//! (`rekindle-records` `lease::max_subkey_bytes`) is tested against it:
//!
//! - a subkey's value is at most `min(MAX_SUBKEY_SIZE, MAX_RECORD_DATA_SIZE
//!   / subkey count)` bytes, the data's own length (`schema.rs:56-65` for
//!   DFLT, whose subkey count is `o_cnt`, `:97-104` for SMPL;
//!   `types/mod.rs:14-16`, `encrypted_value_data.rs:13`,
//!   `value_data.rs:102-104`);
//! - the rule is checked at every hop: before a set leaves the node (a
//!   failure is `Generic` "failed schema validation", `set_value.rs:233-242`),
//!   at the replica (an invalid message, nothing stored, `set_value.rs:759`),
//!   on a get answer (the answer is invalid and skipped, `get_value.rs:385`)
//!   and on a watched value (the change is invalid and dropped,
//!   `watch_value.rs:1934-1946`);
//! - a record's subkeys together hold at most `MAX_RECORD_DATA_SIZE` bytes
//!   in a store: a store refusing it is `Internal` "record exceeds maximum
//!   data size" (`record_store/record.rs:105-129`). The per-subkey rule
//!   implies it; it is checked where Veilid checks it.

use rekindle_records::port::{RecordSchema, RecordValue};

/// `MAX_SUBKEY_SIZE` (`types/mod.rs:14`, `encrypted_value_data.rs:13`).
pub(crate) const MAX_SUBKEY_SIZE: usize = 32_768;
/// `MAX_RECORD_DATA_SIZE` (`types/mod.rs:16`).
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
mod tests {
    use std::sync::Arc;

    use rekindle_crypto::keypair_text::keypair_text_from_signing;
    use rekindle_records::port::{DhtBackend, DhtErrorKind, SchemaMember, WriterKey};
    use rekindle_secrets::ed25519_dalek::SigningKey;

    use super::*;
    use crate::records::dht::SimDht;
    use crate::Netsim;

    fn member_schema(members: usize) -> (RecordSchema, String) {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let keypair = keypair_text_from_signing(&signing);
        let mut list = vec![SchemaMember {
            key: signing.verifying_key().to_bytes(),
            subkeys: 1,
        }];
        for i in 1..members {
            let mut key = [0u8; 32];
            key[..8].copy_from_slice(&(i as u64).to_le_bytes());
            list.push(SchemaMember { key, subkeys: 1 });
        }
        let schema = RecordSchema::Smpl {
            owner_subkeys: 0,
            members: list,
        };
        (schema, keypair)
    }

    #[test]
    fn the_cap_is_veilids() {
        assert_eq!(
            max_value_len(&RecordSchema::Dflt { owner_subkeys: 1 }),
            32_768
        );
        assert_eq!(
            max_value_len(&RecordSchema::Dflt { owner_subkeys: 64 }),
            16_384
        );
        let (schema, _) = member_schema(255);
        assert_eq!(max_value_len(&schema), 4_112, "1 MiB / 255 subkeys");
    }

    /// A 255-member SMPL record takes 4,112 B in a member's subkey and
    /// refuses 4,113 B before the set leaves the node, as `Generic`
    /// "failed schema validation"; a modified client's oversize write is
    /// refused by every replica; a get skips an oversize answer.
    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn a_255_member_record_holds_4112_bytes_a_subkey() {
        let (sim, nodes) = Netsim::nodes(2, 61);
        let dht = SimDht::new(0, Arc::clone(&sim.net), Arc::clone(&nodes[0].rng));
        let (schema, member) = member_schema(255);
        let record = dht.create(&schema, None).await.unwrap();
        let writer = WriterKey {
            keypair: member.clone(),
            public: String::new(),
        };
        dht.set(&record.key, 0, vec![1; 4_112], Some(&writer))
            .await
            .unwrap();
        let err = dht
            .set(&record.key, 0, vec![2; 4_113], Some(&writer))
            .await
            .unwrap_err();
        assert_eq!(err.kind, DhtErrorKind::Generic);
        assert!(err.message.contains("failed schema validation"), "{err}");

        assert!(sim
            .forge_write(&record.key, 0, vec![3; 4_113], &member)
            .is_err());
        assert!(sim
            .replica_values(&record.key, 0)
            .iter()
            .flatten()
            .all(|v| v.data.len() == 4_112));

        // An oversize value a replica answers is invalid: the get skips it.
        {
            let mut inner = sim.net.inner.lock();
            let replicas = &mut inner.records.get_mut(&record.key).unwrap().replicas;
            for r in replicas.iter_mut() {
                if let Some(v) = r.values[0].as_mut() {
                    v.seq = 9;
                    v.data = vec![4; 4_113];
                }
            }
        }
        let other = SimDht::new(1, Arc::clone(&sim.net), Arc::clone(&nodes[1].rng));
        other.open(&record.key, None).await.unwrap();
        assert_eq!(other.get(&record.key, 0, true).await.unwrap(), None);
    }

    #[test]
    fn a_store_refuses_past_the_record_total() {
        let schema = RecordSchema::Dflt { owner_subkeys: 32 };
        let full = RecordValue {
            seq: 0,
            data: vec![0; 32_768],
            writer: String::new(),
        };
        let values = vec![Some(full.clone()); 32];
        assert!(
            admits(&schema, &values, 0, &full),
            "32 full subkeys are 1 MiB"
        );
        let one_more = RecordValue {
            data: vec![0; 32_769],
            ..full
        };
        assert!(!within_total(&values, 0, &one_more));
    }
}
