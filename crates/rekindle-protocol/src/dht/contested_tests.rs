//! Channel posts and the profile through a real pool whose sets settle on
//! ours over a rival at our seq (Veilid 0.5.8 `set_value.rs:753-760`, plan
//! VU1 decision 4): the write is stored, not superseded.

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use rekindle_codec::community::channel_record::ChannelReaction;
use rekindle_record_pool::test_support::{pool_on, Contest, MemDht};
use rekindle_records::port::{RecordPort, RecordSchema, SetOutcome};
use rekindle_types::id::PseudonymKey;

use super::community::channel_record::{write_member_reaction, AppendOutcome};
use super::profile::{create_profile, ProfileFields};

/// `channel_record/write.rs`'s compare-and-swap loop: before VU1 it merged
/// "newer" (our own page) and rewrote it until `CAS_ROUNDS` ran out, then
/// reported a post that is on the network as `NotStored`.
#[tokio::test]
async fn a_channel_post_onto_a_contested_slot_is_stored() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    let schema = RecordSchema::Dflt { owner_subkeys: 64 };
    let (lease, channel, _) = RecordPort::create(&*pool, &schema, None).await.unwrap();
    pool.release(lease).await;
    let signing = SigningKey::from_bytes(&[9; 32]);
    let author = PseudonymKey(signing.verifying_key().to_bytes());
    dht.contest(Contest::Ours);
    let reaction = ChannelReaction {
        message_id: "msg-1".into(),
        expression: "👍".into(),
        added: true,
        lamport: 1,
    };
    let outcome = write_member_reaction(
        &*pool,
        &channel,
        0,
        "slot-writer",
        author,
        &signing,
        &reaction,
    )
    .await
    .expect("the post is stored");
    assert!(matches!(outcome, AppendOutcome::Stored), "{outcome:?}");
}

/// `profile.rs`: a profile write that settles on ours is not a miss (it
/// logged "another device won" and was counted as the first miss).
#[tokio::test]
async fn a_profile_published_onto_contested_subkeys_is_landed() {
    let dht = Arc::new(MemDht::default());
    let pool = pool_on(Arc::clone(&dht));
    dht.contest(Contest::Ours);
    let fields = ProfileFields {
        display_name: "Ida",
        status_message: "hi",
        prekey_records: None,
        route_blob: b"route",
    };
    let (_, _, _, outcome) = create_profile(&*pool, fields).await.unwrap();
    assert_eq!(outcome, SetOutcome::Landed);
}
