//! The community anchor `A` and a `MemberRecord` `M(p)` (plan E3.2) over the
//! production pool: only the record's owner writes it, and `owner_of`
//! answers the owner the record's descriptor names, on the creator and on
//! a node that opens it from the network (`storage_manager/
//! create_record.rs:88-104`, `record_key.rs:67-94`, fork 0cd847de: the
//! record key hashes the owner and the descriptor is signed by it).

use rekindle_records::port::{PortError, RecordPort, SetOutcome};
use rekindle_records::schema;
use rekindle_types::dht_layout::{community_anchor, member_record};

use crate::records::public_text;
use crate::tests::keypair;
use crate::Netsim;

/// A foreign key's write to `subkey` fails; the owner's lands.
async fn only_the_owner_writes(
    port: &dyn RecordPort,
    lease: rekindle_records::lease::LeaseId,
    subkey: u32,
) {
    let (foreign, _) = keypair(10);
    assert!(matches!(
        port.set(lease, subkey, b"forged".to_vec(), Some(&foreign))
            .await,
        Err(PortError::Dht(_))
    ));
    assert_eq!(
        port.set(lease, subkey, b"owned".to_vec(), None)
            .await
            .unwrap(),
        SetOutcome::Landed
    );
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_foreign_key_fails_against_the_anchor_and_owner_of_names_its_owner() {
    let (_sim, nodes) = Netsim::nodes(2, 91);
    let (a, b) = (
        nodes[0].ports.records.start_session().unwrap(),
        nodes[1].ports.records.start_session().unwrap(),
    );
    let (lease, key, k_a) = a.create(&schema::community_anchor(), None).await.unwrap();
    let owner = public_text(&crate::records::writer_public(&k_a).unwrap());
    assert_eq!(a.owner_of(lease).unwrap(), owner);
    only_the_owner_writes(a.as_ref(), lease, community_anchor::GENESIS_CERT).await;
    a.release(lease).await;
    assert!(matches!(a.owner_of(lease), Err(PortError::LeaseNotHeld(_))));
    let theirs = b.acquire(&key, None).await.unwrap();
    assert_eq!(b.owner_of(theirs).unwrap(), owner);
    let (foreign, _) = keypair(10);
    assert!(matches!(
        b.set(
            theirs,
            community_anchor::GENESIS_CERT,
            b"x".to_vec(),
            Some(&foreign)
        )
        .await,
        Err(PortError::Dht(_))
    ));
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_foreign_key_fails_against_a_member_record_and_owner_of_names_its_owner() {
    let (_sim, nodes) = Netsim::nodes(2, 92);
    let (a, b) = (
        nodes[0].ports.records.start_session().unwrap(),
        nodes[1].ports.records.start_session().unwrap(),
    );
    let (k_m, k_m_public) = keypair(9);
    let (lease, key, owner_pair) = a
        .create(&schema::member_record(), Some(&k_m))
        .await
        .unwrap();
    assert_eq!(owner_pair, k_m);
    assert_eq!(a.owner_of(lease).unwrap(), public_text(&k_m_public));
    for subkey in [
        member_record::CARD,
        member_record::BEAT,
        member_record::HISTORY_AD,
    ] {
        only_the_owner_writes(a.as_ref(), lease, subkey).await;
    }
    let theirs = b.acquire(&key, None).await.unwrap();
    assert_eq!(b.owner_of(theirs).unwrap(), public_text(&k_m_public));
    let (foreign, foreign_public) = keypair(10);
    assert_ne!(b.owner_of(theirs).unwrap(), public_text(&foreign_public));
    assert!(matches!(
        b.set(
            theirs,
            member_record::FRONTIER,
            b"x".to_vec(),
            Some(&foreign)
        )
        .await,
        Err(PortError::Dht(_))
    ));
}
