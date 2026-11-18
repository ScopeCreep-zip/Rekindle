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

/// One card read's simulated round trip.
const RTT: std::time::Duration = std::time::Duration::from_millis(500);

/// Plan E3.2 S7c.9: a joiner's first rebuild reads every member's card;
/// the reads run eight at a time (half of Veilid's gets, S7c.8 audit (8)),
/// in the members' order, so 40 cards cost five round trips, where one
/// read at a time cost forty.
#[tokio::test(start_paused = true)]
async fn card_reads_run_eight_in_flight_in_member_order() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use rekindle_types::id::PseudonymKey;

    use super::{read_each, CARD_READS_IN_FLIGHT};

    const MEMBERS: u8 = 40;
    let pseudonyms: Vec<PseudonymKey> = (0..MEMBERS).map(|i| PseudonymKey([i; 32])).collect();
    let (in_flight, most) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let (in_flight, most) = (&in_flight, &most);
    let mut reads = Vec::new();
    for pseudonym in &pseudonyms {
        reads.push((pseudonym, async move {
            let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(RTT).await;
            in_flight.fetch_sub(1, Ordering::SeqCst);
            pseudonym.0[0]
        }));
    }
    let started = tokio::time::Instant::now();
    let read = read_each(reads).await;
    let elapsed = started.elapsed();
    assert_eq!(most.load(Ordering::SeqCst), CARD_READS_IN_FLIGHT);
    let order: Vec<u8> = read
        .iter()
        .map(|(p, seen)| {
            assert_eq!(p.0[0], *seen);
            *seen
        })
        .collect();
    assert_eq!(order, (0..MEMBERS).collect::<Vec<_>>());
    let rounds = usize::from(MEMBERS).div_ceil(CARD_READS_IN_FLIGHT);
    assert_eq!(elapsed, RTT * u32::try_from(rounds).unwrap());
}

/// Plan E3.2 S7c.9: a refresh never reads our own card (a joiner's first
/// rebuild runs before it writes it, and a forced read of the unwritten
/// subkey is a fanout to its 10 s timeout); it reads every other member
/// whose name we lack.
#[test]
fn a_refresh_reads_every_card_but_ours_and_the_named() {
    use rekindle_governance::state::{GovernanceState, MemberState};
    use rekindle_types::id::PseudonymKey;

    let mut state = GovernanceState::default();
    for i in 0..3u8 {
        state.members.insert(
            PseudonymKey([i; 32]),
            MemberState {
                member_record: format!("M{i}"),
                member_record_owner: format!("o{i}"),
                joined: rekindle_governance::op::OpStamp::MIN,
            },
        );
    }
    let own = PseudonymKey([1; 32]);
    let named = PseudonymKey([2; 32]);
    let read: Vec<&str> = super::cards_to_read(&state, "C", Some(&own), |p| *p == named)
        .iter()
        .map(|m| m.record)
        .collect();
    assert_eq!(read, vec!["M0"]);
}
