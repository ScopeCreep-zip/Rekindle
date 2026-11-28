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

use rekindle_presence::community::{beat_is_live, BEAT_LIVENESS_MS};

use super::{write_beat, write_card, OwnCardText, OwnProfile, OwnRecord};

fn ida() -> OwnCardText {
    OwnCardText {
        display_name: Some("Ida".into()),
        custom_status: None,
        game: None,
    }
}

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
        status: SessionStatus::Online,
        beat_seq: 1,
        now_ms: 1_000,
        route: Vec::new(),
    };
    dht.contest(Contest::Ours);
    write_card(&*pool, &own, &ida(), &[3; 32])
        .await
        .expect("a card that settled on ours is stored");
    write_beat(&*pool, &own, &profile)
        .await
        .expect("a beat that settled on ours is stored");
    dht.contest(Contest::None);
    // A beat's padding is fresh at each encode, so the identical rewrite
    // is the card's.
    write_card(&*pool, &own, &ida(), &[3; 32])
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

    use rekindle_types::governance::{AdmissionMode, GenesisCert};

    let mut state = GovernanceState::default();
    let mut members = Vec::new();
    for i in 0..3u8 {
        let p = SigningKey::from_bytes(&[i; 32]);
        let d = SigningKey::from_bytes(&[i + 10; 32]);
        let member = PseudonymKey(p.verifying_key().to_bytes());
        let record = format!("M{i}");
        let owner = format!("o{i}");
        state.members.insert(
            member.clone(),
            MemberState {
                member_record: record.clone(),
                member_record_owner: owner.clone(),
                joined: rekindle_governance::op::OpStamp::MIN,
            },
        );
        let endorsement = crate::device_signer::first_endorsement(&p, "C", &d, &record, &owner);
        state.devices.seed_genesis(
            "C",
            &GenesisCert {
                name: "C".into(),
                policy: AdmissionMode::Open,
                initial_roles: Vec::new(),
                creator_pseudonym: member.clone(),
                creator_member_record: record,
                creator_member_record_owner: owner,
                created_at: 0,
                additional_creators: Vec::new(),
                predecessor: None,
                creator_device: Some(endorsement),
            },
        );
        members.push(member);
    }
    let own = &members[1];
    let named = &members[2];
    let read: Vec<&str> = super::cards_to_read(&state, "C", Some(own), |p| p == named)
        .iter()
        .map(|m| m.record)
        .collect();
    assert_eq!(read, vec!["M0"]);
}

/// Plan E3.4 Q4: a beat is live for 15 minutes from the later of its beat
/// number's time and its arrival, at both edges and under skewed clocks.
#[test]
fn a_beat_is_live_for_15_minutes_from_the_later_of_its_number_and_its_arrival() {
    use rekindle_codec::member_record::{beat_number_at, BEAT_PERIOD_MS};

    assert_eq!(BEAT_LIVENESS_MS, 15 * 60 * 1000);
    let number = 5_871_000;
    let written = number * BEAT_PERIOD_MS;
    assert_eq!(beat_number_at(written + BEAT_PERIOD_MS - 1), number);
    // Read cold: only the number counts, up to the last millisecond.
    assert!(beat_is_live(number, None, written));
    assert!(beat_is_live(number, None, written + BEAT_LIVENESS_MS - 1));
    assert!(!beat_is_live(number, None, written + BEAT_LIVENESS_MS));
    // The writer's clock runs an hour behind ours: its number is an hour
    // old when it arrives, and it is live for 15 minutes from the arrival.
    let arrived = written + 60 * 60 * 1000;
    assert!(beat_is_live(
        number,
        Some(arrived),
        arrived + BEAT_LIVENESS_MS - 1
    ));
    assert!(!beat_is_live(
        number,
        Some(arrived),
        arrived + BEAT_LIVENESS_MS
    ));
    assert!(!beat_is_live(number, None, arrived));
    // The writer's clock runs ahead by a period: the number's time is the
    // later one.
    let early = written - BEAT_PERIOD_MS;
    assert!(beat_is_live(
        number,
        Some(early),
        written + BEAT_LIVENESS_MS - 1
    ));
    assert!(!beat_is_live(
        number,
        Some(early),
        written + BEAT_LIVENESS_MS
    ));
    // No overflow at the extremes.
    assert!(beat_is_live(u64::MAX, None, u64::MAX - 1));
    assert!(!beat_is_live(0, Some(0), BEAT_LIVENESS_MS));
}

/// Plan E3.4 Q4 as amended 2026-11-19: a beat number more than one period
/// ahead of our clock at its arrival counts from the arrival, so a future
/// number cannot keep its member live; read cold, the number alone counts.
#[test]
fn a_beat_number_far_ahead_counts_from_its_arrival() {
    use rekindle_codec::member_record::BEAT_PERIOD_MS;

    let arrived = 5_871_000 * BEAT_PERIOD_MS;
    let hour_ahead = (arrived + 60 * 60 * 1000) / BEAT_PERIOD_MS;
    assert!(beat_is_live(
        hour_ahead,
        Some(arrived),
        arrived + BEAT_LIVENESS_MS - 1
    ));
    assert!(!beat_is_live(
        hour_ahead,
        Some(arrived),
        arrived + 16 * 60 * 1000
    ));
    assert!(!beat_is_live(
        hour_ahead,
        Some(arrived),
        arrived + 55 * 60 * 1000
    ));
    // Just over one period ahead is too far; exactly one period is not.
    let over = (arrived + BEAT_PERIOD_MS) / BEAT_PERIOD_MS + 1;
    assert!(!beat_is_live(
        over,
        Some(arrived),
        arrived + BEAT_LIVENESS_MS
    ));
    let one = (arrived + BEAT_PERIOD_MS) / BEAT_PERIOD_MS;
    assert!(beat_is_live(one, Some(arrived), arrived + BEAT_LIVENESS_MS));
    assert!(!beat_is_live(
        one,
        Some(arrived),
        arrived + BEAT_PERIOD_MS + BEAT_LIVENESS_MS
    ));
    // Cold, the number alone counts.
    assert!(beat_is_live(hour_ahead, None, arrived + 16 * 60 * 1000));
}

/// Plan E3.4 S2 ("Constant rate"): an Invisible member's record gets its
/// card, routes and ops, and no beat; a visible one gets its beat.
#[tokio::test]
async fn an_invisible_member_publishes_its_record_without_a_beat() {
    for (status, beat) in [
        (SessionStatus::Invisible, false),
        (SessionStatus::Away, true),
    ] {
        let dht = Arc::new(MemDht::default());
        let pool = pool_on(Arc::clone(&dht));
        let (lease, record, writer) =
            RecordPort::create(&*pool, &rekindle_records::schema::member_record(), None)
                .await
                .unwrap();
        pool.release(lease).await;
        let pseudonym = SigningKey::from_bytes(&[6; 32]);
        let own = OwnRecord {
            community_id: "community",
            record: &record,
            writer: &writer,
            pseudonym: &pseudonym,
        };
        let profile = OwnProfile {
            status,
            beat_seq: 1,
            now_ms: 1_000,
            route: Vec::new(),
        };
        super::publish_own_record(&*pool, &own, &profile, &[3; 32], async { Ok(()) })
            .await
            .expect("published");
        assert!(dht.value(&record, member_record::CARD).is_some());
        assert_eq!(
            dht.value(&record, member_record::BEAT).is_some(),
            beat,
            "{status:?}"
        );
    }
}

/// Plan E3.4 S6: the game our card carries is what a peer's verified read
/// of the network copy (`decode_member_card` + `card_of`, as the ring's
/// `ring_card_moved` reads it) sees, and a stopped game rewrites the card
/// without one.
#[tokio::test]
async fn a_card_game_reaches_a_peer_and_a_stopped_one_clears() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let (lease, record, writer) =
        RecordPort::create(&*pool, &rekindle_records::schema::member_record(), None)
            .await
            .unwrap();
    pool.release(lease).await;
    let pseudonym = SigningKey::from_bytes(&[6; 32]);
    let own = OwnRecord {
        community_id: "community",
        record: &record,
        writer: &writer,
        pseudonym: &pseudonym,
    };
    let peer_reads = || {
        let value = dht.value(&record, member_record::CARD).expect("a card");
        let at = rekindle_codec::member_record::RecordScope {
            community_id: "community",
            member_record: &record,
        };
        let signed = rekindle_codec::member_record::card::decode_member_card(
            &value.data,
            at,
            &pseudonym.verifying_key().to_bytes(),
        )
        .expect("the card verifies");
        super::card_of(&signed).game
    };
    let game = rekindle_types::presence::GameInfo {
        game_name: "Quake III Arena".into(),
        game_id: Some("4100".into()),
        elapsed_seconds: None,
        server_address: Some("10.0.0.7:27960".into()),
    };
    let playing = OwnCardText {
        game: Some(game.clone()),
        ..ida()
    };
    write_card(&*pool, &own, &playing, &[3; 32]).await.unwrap();
    assert_eq!(
        peer_reads(),
        Some(game),
        "the started game reaches the peer"
    );
    write_card(&*pool, &own, &ida(), &[3; 32]).await.unwrap();
    assert_eq!(peer_reads(), None, "the stopped game clears");
}
