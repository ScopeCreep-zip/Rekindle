//! The profile through a real pool whose sets settle on ours over a rival
//! at our seq (Veilid 0.5.8 `set_value.rs:753-760`, plan
//! VU1 decision 4): the write is stored, not superseded.

use std::sync::Arc;

use rekindle_record_pool::test_support::{pool_on, Contest, MemDht};
use rekindle_records::port::SetOutcome;

use super::profile::{create_profile, ProfileFields};

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
