//! The one receive path of a delivered key (`MekTransfer`, D16; plan E3.1,
//! CB3 glue audit): both hosts hand the payload to
//! `HostGovernance::receive_mek_transfer`, so every check the desktop's
//! retired chain skipped (membership, removal, departed rotator, the
//! generation the transfer names) is the Host's and is covered here.

use rekindle_codec::community::envelope::MekTransferPayload;
use rekindle_transport::crypto::mek::{wrap_mek, Mek};
use rekindle_transport::crypto::pseudonym::derive_community_pseudonym;
use rekindle_types::channel_keys::KeyScope;

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::governance::mek_transfer::MekTransferRefusal;
use crate::host::Host;

const ROTATOR_SECRET: [u8; 32] = [9; 32];

/// A transfer of generation `inside` wrapped by the rotator for our
/// pseudonym in `community`, naming generation `named`.
fn transfer(host: &Host, community: &str, inside: u64, named: u64) -> MekTransferPayload {
    let secret = host.ctx.identity_secret().expect("unlocked");
    let ours = derive_community_pseudonym(&secret, community);
    let rotator = derive_community_pseudonym(&ROTATOR_SECRET, community);
    let key = Mek::from_bytes([7; 32], inside);
    let wrapped = wrap_mek(
        &rotator,
        &ours.verifying_key().to_bytes(),
        &key.to_wire_bytes(),
    )
    .expect("wrapped");
    MekTransferPayload {
        community_id: community.into(),
        channel_id: None,
        generation: named,
        sender_pseudonym: hex::encode(rotator.verifying_key().to_bytes()),
        wrapped_mek: wrapped,
        transfer_id: vec![1; 16],
    }
}

fn held(host: &Host) -> Option<u64> {
    host.ctx
        .mek_cache
        .read()
        .current("c1", KeyScope::Community)
        .map(Mek::generation)
}

#[tokio::test]
async fn a_members_transfer_installs_and_is_acked() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    let ack = host
        .governance()
        .receive_mek_transfer(&transfer(&host, "c1", 2, 2))
        .expect("acked");
    assert!(!ack.is_empty(), "the answer is an encoded MekTransferAck");
    assert_eq!(held(&host), Some(2), "the key is installed");
    host.shutdown().await.expect("the host stops");
}

#[tokio::test]
async fn a_transfer_for_a_community_we_are_not_in_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    create_ann(&host).await;
    let answer = host
        .governance()
        .receive_mek_transfer(&transfer(&host, "c1", 2, 2));
    assert_eq!(answer, Err(MekTransferRefusal::NotAMember));
    assert_eq!(held(&host), None);
    host.shutdown().await.expect("the host stops");
}

#[tokio::test]
async fn a_transfer_after_our_removal_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    if let Some(community) = host.ctx.communities.map().write().get_mut("c1") {
        community.own_removal = Some(crate::host::community::OwnRemoval {
            banned: false,
            by: None,
            by_name: None,
            reason: None,
        });
    }
    let answer = host
        .governance()
        .receive_mek_transfer(&transfer(&host, "c1", 2, 2));
    assert_eq!(answer, Err(MekTransferRefusal::Removed));
    assert_eq!(held(&host), None, "nothing is stored after a removal");
    host.shutdown().await.expect("the host stops");
}

#[tokio::test]
async fn a_key_of_another_generation_than_named_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let ann = create_ann(&host).await;
    seed_registry_community(&host, &ann.public_key).await;
    let answer = host
        .governance()
        .receive_mek_transfer(&transfer(&host, "c1", 5, 2));
    assert_eq!(answer, Err(MekTransferRefusal::UnwrapFailed));
    assert_eq!(held(&host), None);
    host.shutdown().await.expect("the host stops");
}
