//! `M`/6 through a real pool (plan E3.2 S7c.8 item (4)): an invite names
//! the inviter's record, and after the inviter's route is replaced the
//! joiner's read returns the new route, not the one the record held when
//! the invite was minted.

use std::sync::Arc;

use rekindle_record_pool::test_support::{pool_on, MemDht};
use rekindle_records::port::RecordPort;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::id::PseudonymKey;
use rekindle_types::presence::SessionStatus;

use super::{read_member_route, write_routes};
use crate::member_card::{publish_own_record, OwnProfile, OwnRecord};

#[tokio::test]
async fn a_joiner_reads_the_inviters_route_after_it_was_replaced() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let (lease, record, writer) =
        RecordPort::create(&*pool, &rekindle_records::schema::member_record(), None)
            .await
            .unwrap();
    pool.release(lease).await;
    let pseudonym = SigningKey::from_bytes(&[5; 32]);
    let inviter = PseudonymKey(pseudonym.verifying_key().to_bytes());
    let own = OwnRecord {
        community_id: "community",
        record: &record,
        writer: &writer,
        pseudonym: &pseudonym,
    };
    // The record as published when the invite was minted: route A.
    let profile = OwnProfile {
        display_name: "Ida".into(),
        status: SessionStatus::Online,
        now_ms: 1_000,
        route: b"route-A".to_vec(),
    };
    publish_own_record(&*pool, &own, &profile, &[3; 32], async { Ok(()) })
        .await
        .unwrap();
    let read = read_member_route(&*pool, "community", &record, &inviter).await;
    assert_eq!(read.unwrap().as_deref(), Some(&b"route-A"[..]));

    // Route A dies; the route publisher writes route B.
    write_routes(&*pool, &own, b"route-B", 2_000).await.unwrap();
    let read = read_member_route(&*pool, "community", &record, &inviter).await;
    assert_eq!(
        read.unwrap().as_deref(),
        Some(&b"route-B"[..]),
        "the joiner resolves the inviter's current route"
    );

    // Signed by another pseudonym, or read for another community: refused.
    let other = PseudonymKey(SigningKey::from_bytes(&[6; 32]).verifying_key().to_bytes());
    assert!(read_member_route(&*pool, "community", &record, &other)
        .await
        .is_err());
    assert!(read_member_route(&*pool, "elsewhere", &record, &inviter)
        .await
        .is_err());
}
