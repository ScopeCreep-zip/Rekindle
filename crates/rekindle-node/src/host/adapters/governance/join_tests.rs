//! The joiner's member record: §4.4's publish order (`M`/0 before `M`/2,
//! the beat last) and the reader's owner pin and signature check.

use rekindle_governance::fixture;
use rekindle_governance_runtime::member_card::{
    publish_own_record, read_beat, read_card, write_card, MemberRecordRef, OwnProfile, OwnRecord,
};
use rekindle_records::port::RecordPort;
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::dht_layout::member_record::{BEAT, CARD, GOV_HEAD};
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::PseudonymKey;
use rekindle_types::presence::SessionStatus;

use super::super::ops::append_op_once;

const COMMUNITY: &str = "community";

/// A fresh member record: its key, writer keypair text and owner public key.
async fn record(port: &dyn RecordPort) -> (String, String, String) {
    let (lease, key, writer) = port
        .create(&rekindle_records::schema::member_record(), None)
        .await
        .unwrap();
    let owner = port.owner_of(lease).unwrap();
    port.release(lease).await;
    (key, writer, owner)
}

fn profile() -> OwnProfile {
    OwnProfile {
        display_name: "Joiner".into(),
        status: SessionStatus::Online,
        now_ms: 1_000,
    }
}

fn joined_op(author: &PseudonymKey) -> Vec<u8> {
    let op = fixture::op(
        author,
        0,
        &[],
        1,
        GovernanceEntry::CommunityMeta {
            name: Some("Join".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
        },
    )
    .op()
    .clone();
    rekindle_codec::governance_op::encode_op(&op).unwrap()
}

#[tokio::test]
async fn the_card_lands_before_op_zero_and_the_beat_after() {
    let (sim, mut nodes) = rekindle_netsim::Netsim::nodes(1, 83);
    let node = nodes.remove(0);
    let _pump = sim.start();
    sim.quiesce().await;
    let port = node.ports.records.start_session().unwrap();
    let port = &*port;
    let (key, writer, owner) = record(port).await;
    let signing = SigningKey::from_bytes(&[5; 32]);
    let pseudonym = PseudonymKey(signing.verifying_key().to_bytes());
    let own = OwnRecord {
        community_id: COMMUNITY,
        record: &key,
        writer: &writer,
        pseudonym: &signing,
    };
    let op = joined_op(&pseudonym);
    let publish_ops = async {
        // When op #0 is appended, `M`/0 is there and `M`/1 is not yet.
        assert!(port.read_once(&key, CARD, false).await.unwrap().is_some());
        assert!(port.read_once(&key, BEAT, false).await.unwrap().is_none());
        assert!(port
            .read_once(&key, GOV_HEAD, false)
            .await
            .unwrap()
            .is_none());
        append_op_once(port, &key, &writer, op).await.map(drop)
    };
    publish_own_record(port, &own, &profile(), &[9; 32], publish_ops)
        .await
        .unwrap();
    let member = MemberRecordRef {
        community_id: COMMUNITY,
        pseudonym: &pseudonym,
        record: &key,
        owner: &owner,
    };
    let card = read_card(port, &member).await.unwrap().unwrap();
    assert_eq!(card.card.display_name.as_deref(), Some("Joiner"));
    assert_eq!(
        card.card.hpke_key,
        rekindle_secrets::hpke::public_key(&[9; 32])
    );
    let beat = read_beat(port, &member).await.unwrap().unwrap();
    assert_eq!(beat.status, SessionStatus::Online);
    assert!(beat.extras.is_none());
}

#[tokio::test]
async fn a_card_under_another_owner_or_signer_is_refused() {
    let (sim, mut nodes) = rekindle_netsim::Netsim::nodes(1, 84);
    let node = nodes.remove(0);
    let _pump = sim.start();
    sim.quiesce().await;
    let port = node.ports.records.start_session().unwrap();
    let port = &*port;
    let signing = SigningKey::from_bytes(&[5; 32]);
    let pseudonym = PseudonymKey(signing.verifying_key().to_bytes());
    let (_, _, owner) = record(port).await;
    // A forger writes a card in its own record, signed by the member's
    // pseudonym key it does not hold here, and names the member's record
    // owner: the owner pin refuses it.
    let (forged, forged_writer, _) = record(port).await;
    let own = OwnRecord {
        community_id: COMMUNITY,
        record: &forged,
        writer: &forged_writer,
        pseudonym: &signing,
    };
    write_card(port, &own, &profile(), &[9; 32]).await.unwrap();
    let pinned_elsewhere = MemberRecordRef {
        community_id: COMMUNITY,
        pseudonym: &pseudonym,
        record: &forged,
        owner: &owner,
    };
    assert!(read_card(port, &pinned_elsewhere).await.is_err());
    // A card in the right record signed by another pseudonym.
    let other = SigningKey::from_bytes(&[6; 32]);
    let (own_key, own_writer, own_owner) = record(port).await;
    let wrong_signer = OwnRecord {
        community_id: COMMUNITY,
        record: &own_key,
        writer: &own_writer,
        pseudonym: &other,
    };
    write_card(port, &wrong_signer, &profile(), &[9; 32])
        .await
        .unwrap();
    let member = MemberRecordRef {
        community_id: COMMUNITY,
        pseudonym: &pseudonym,
        record: &own_key,
        owner: &own_owner,
    };
    assert!(read_card(port, &member).await.is_err());
}
