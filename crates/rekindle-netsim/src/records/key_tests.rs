//! A record key binds its owner, schema and encryption key, as Veilid
//! derives it (`storage_manager/record_key.rs:67-94`).

use std::sync::Arc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rekindle_records::port::{DhtBackend, RecordSchema, SchemaMember};

use super::dht::SimDht;
use super::record_key_text;
use crate::Netsim;

fn bytes32(text: &str) -> [u8; 32] {
    URL_SAFE_NO_PAD
        .decode(text)
        .unwrap()
        .try_into()
        .expect("32 bytes")
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_record_key_re_derives_from_its_owner_schema_and_encryption_key() {
    let (sim, nodes) = Netsim::nodes(1, 7);
    let a = SimDht::new(0, Arc::clone(&sim.net), Arc::clone(&nodes[0].rng));
    let schemas = [
        RecordSchema::Dflt { owner_subkeys: 8 },
        RecordSchema::Smpl {
            owner_subkeys: 1,
            members: vec![SchemaMember {
                key: [9u8; 32],
                subkeys: 2,
            }],
        },
    ];
    for schema in schemas {
        let record = a.create(&schema, None).await.unwrap();
        let mut parts = record.key.strip_prefix("VLD0:").unwrap().split(':');
        let (_opaque, secret) = (parts.next().unwrap(), parts.next().unwrap());
        let encryption_key = bytes32(secret);
        let owner = bytes32(record.owner_public.strip_prefix("VLD0:").unwrap());
        assert_eq!(
            record_key_text(&owner, &schema, &encryption_key),
            record.key
        );

        let other = [owner[0] ^ 1; 32];
        assert_ne!(
            record_key_text(&other, &schema, &encryption_key),
            record.key,
            "a different owner gives a different key"
        );
        let wider = RecordSchema::Dflt { owner_subkeys: 9 };
        assert_ne!(record_key_text(&owner, &wider, &encryption_key), record.key);
    }
}
