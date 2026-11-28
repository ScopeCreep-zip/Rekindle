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
    let writer = WriterKey::new(
        String::new(),
        rekindle_secrets::record_writer::RecordWriterSecret::from_text(&member),
    );
    dht.set(&record.key, 0, vec![1; 4_112], Some(&writer))
        .await
        .unwrap();
    // The set's RPCs still in flight at its return land (`records/fanout.rs`).
    tokio::time::sleep(crate::latency::SET_RPC.max).await;
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
    other
        .open(&record.key, None, rekindle_records::port::OpenIntent::Hold)
        .await
        .unwrap();
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
