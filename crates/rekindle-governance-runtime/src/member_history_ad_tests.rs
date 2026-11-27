//! `M`/5 through a real pool (plan E3.4 S7): the ad a member writes is read
//! back from its record, owner pinned, and opens only with the key of its
//! generation under the writer's pseudonym.

use std::sync::Arc;

use rekindle_codec::member_record::history_ad::{decode_history_ad, HISTORY_AD_MAX_RANGES};
use rekindle_codec::member_record::RecordScope;
use rekindle_record_pool::test_support::{pool_on, MemDht};
use rekindle_records::port::RecordPort;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::keys::MediaEncryptionKey;
use rekindle_types::id::{ChannelId, PseudonymKey};
use rekindle_types::presence::fixture::history_range;
use rekindle_types::presence::HistoryRange;
use rekindle_utils::random::OsRandom;

use super::{advertised, read_history_ad, write_history_ad};
use crate::member_card::{MemberRecordRef, OwnRecord};

#[test]
fn an_ad_carries_the_most_recently_active_channels_up_to_its_bound() {
    let ranges: Vec<HistoryRange> = (0..=u8::try_from(HISTORY_AD_MAX_RANGES).unwrap())
        .map(|i| history_range(i, u64::from(i) + 10))
        .collect();
    let picked = advertised(ranges);
    assert_eq!(picked.len(), HISTORY_AD_MAX_RANGES);
    assert_eq!(picked[0].channel_id, ChannelId([64; 16]), "newest first");
    assert!(
        picked.iter().all(|r| r.channel_id != ChannelId([0; 16])),
        "the least recently active channel is the one left out"
    );
}

#[tokio::test]
async fn a_written_ad_is_read_back_from_the_member_record() {
    let pool = pool_on(Arc::new(MemDht::default()));
    let (lease, record, writer) =
        RecordPort::create(&*pool, &rekindle_records::schema::member_record(), None)
            .await
            .unwrap();
    let owner = pool.owner_of(lease).unwrap();
    pool.release(lease).await;
    let pseudonym = SigningKey::from_bytes(&[5; 32]);
    let member = PseudonymKey(pseudonym.verifying_key().to_bytes());
    let own = OwnRecord {
        community_id: "community",
        record: &record,
        writer: &writer,
        pseudonym: &pseudonym,
    };
    let at = MemberRecordRef {
        community_id: "community",
        pseudonym: &member,
        record: &record,
        owner: &owner,
    };
    assert_eq!(read_history_ad(&*pool, &at).await.unwrap(), None);

    let key = MediaEncryptionKey::from_bytes([2; 32], 3);
    let ranges = advertised(vec![history_range(1, 5), history_range(2, 9)]);
    write_history_ad(&*pool, &own, &ranges, &key, &OsRandom)
        .await
        .unwrap();
    let bytes = read_history_ad(&*pool, &at).await.unwrap().expect("an ad");
    let scope = RecordScope {
        community_id: "community",
        member_record: &record,
    };
    assert_eq!(
        decode_history_ad(&bytes, &key, scope, &member.0).unwrap(),
        ranges
    );
    let rotated = MediaEncryptionKey::from_bytes([4; 32], 4);
    assert!(
        decode_history_ad(&bytes, &rotated, scope, &member.0).is_err(),
        "an ad sealed before a rotation does not open with the new key"
    );

    // A record whose owner is not the one `MemberJoined` names: refused.
    let forged = MemberRecordRef {
        owner: "VLD0:not-the-owner",
        ..at
    };
    assert!(read_history_ad(&*pool, &forged).await.is_err());
}
