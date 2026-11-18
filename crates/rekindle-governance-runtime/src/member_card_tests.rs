//! The member record's writes through a real pool (plan VU1, #1): a write
//! that settles on ours over a rival (Veilid 0.5.8 `set_value.rs:753-760`)
//! and an identical rewrite (`Unchanged`, refused before VU1 too) are
//! stored.

use std::sync::Arc;

use rekindle_record_pool::test_support::{pool_on, Contest, MemDht};
use rekindle_records::port::RecordPort;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::dht_layout::member_record;
use rekindle_types::presence::SessionStatus;

use super::{write_beat, write_card, OwnProfile, OwnRecord};

#[tokio::test]
async fn member_record_writes_onto_contested_or_unchanged_subkeys_are_stored() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let (lease, record, writer) =
        RecordPort::create(&*pool, &rekindle_records::schema::member_record(), None)
            .await
            .unwrap();
    pool.release(lease).await;
    let pseudonym = SigningKey::from_bytes(&[5; 32]);
    let own = OwnRecord {
        community_id: "community",
        record: &record,
        writer: &writer,
        pseudonym: &pseudonym,
    };
    let profile = OwnProfile {
        display_name: "Ida".into(),
        status: SessionStatus::Online,
        now_ms: 1_000,
        route: Vec::new(),
    };
    dht.contest(Contest::Ours);
    write_card(&*pool, &own, &profile, &[3; 32])
        .await
        .expect("a card that settled on ours is stored");
    write_beat(&*pool, &own, &profile)
        .await
        .expect("a beat that settled on ours is stored");
    dht.contest(Contest::None);
    write_beat(&*pool, &own, &profile)
        .await
        .expect("an identical rewrite (Unchanged) is stored");
    assert!(dht.value(&record, member_record::CARD).is_some());
}
