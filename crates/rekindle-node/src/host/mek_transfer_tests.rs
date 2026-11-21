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

/// The inviter's side of a key transfer (D16, D22; review S7m part 1 F2,
/// F3): a host whose community `c1` admits a joiner, holding the key at
/// generation 3, with the joiner's row queued. Returns the joiner.
async fn queued_for_a_joiner(host: &Host, owner: &str) -> String {
    use rekindle_governance::op::OpStamp;
    use rekindle_types::id::PseudonymKey;

    seed_registry_community(host, owner).await;
    let joiner = hex::encode(
        derive_community_pseudonym(&[5; 32], "c1")
            .verifying_key()
            .to_bytes(),
    );
    {
        let mut map = host.ctx.communities.map().write();
        let c = map.get_mut("c1").unwrap();
        c.my_pseudonym_key = Some(hex::encode(
            derive_community_pseudonym(&host.ctx.identity_secret().unwrap(), "c1")
                .verifying_key()
                .to_bytes(),
        ));
        let mut gov = rekindle_governance::state::GovernanceState::default();
        gov.members.insert(
            PseudonymKey::from_hex_lossy(&joiner),
            rekindle_governance::state::MemberState {
                member_record: "r".into(),
                member_record_owner: "o".into(),
                joined: OpStamp::MIN,
            },
        );
        c.governance_state = Some(gov);
    }
    let cache = crate::daemon::mek_rotation::MekCacheAdapter::new(std::sync::Arc::clone(
        &host.ctx.mek_cache,
    ));
    rekindle_mek_rotation::ChannelMekCache::insert(
        &cache,
        "c1",
        KeyScope::Community,
        Mek::from_bytes([7; 32], 3),
    );
    deliver(host, &joiner).await;
    assert_eq!(rows(host).await.len(), 1, "the joiner's row is queued");
    joiner
}

/// The inviter's half of the joiner's bootstrap (`deliver_key_to_joiner`).
async fn deliver(host: &Host, joiner: &str) {
    use rekindle_governance::op::OpStamp;
    use rekindle_types::id::PseudonymKey;
    let deps = crate::daemon::mek_rotation::DaemonMekAdapter::new(std::sync::Arc::clone(&host.ctx));
    let bootstrap = rekindle_governance_runtime::BootstrapJoiner {
        pseudonym: joiner.to_owned(),
        member_joined: rekindle_types::governance::OpRef {
            op_hash: OpStamp::MIN.hash,
            author: PseudonymKey::from_hex_lossy(joiner),
            author_record: "r".into(),
            author_seq: 1,
        },
        member_joined_op: Vec::new(),
        route: b"route".to_vec(),
    };
    host.governance()
        .deliver_key_to_joiner(&deps, "c1", &bootstrap)
        .await;
}

async fn rows(host: &Host) -> Vec<rekindle_outbox::OutboxRow> {
    let transfers = host.ctx.outbox.mek_transfers().expect("unlocked");
    transfers.rows("c1").await.expect("rows")
}

/// The merged removal of `member` (a kick, a ban or its leave).
fn merge_removal(host: &Host, member: &str) {
    let mut map = host.ctx.communities.map().write();
    let gov = map
        .get_mut("c1")
        .unwrap()
        .governance_state
        .as_mut()
        .unwrap();
    gov.members
        .remove(&rekindle_types::id::PseudonymKey::from_hex_lossy(member));
}

async fn attempt(host: &Host, row: &rekindle_outbox::OutboxRow) -> rekindle_outbox::AttemptOutcome {
    use crate::host::adapters::outbox::StreamSender as _;
    crate::host::adapters::outbox::mek_transfer::MekTransferSender(host.governance())
        .send(row)
        .await
}

/// Review S7m part 1 F2 (D22, ADR 0018): a row at generation 3 that a
/// rotation to 4 overtakes before it is sent settles superseded, and the
/// rotation queued the joiner one row at 4, which is the one attempted.
#[tokio::test]
async fn a_rotation_supersedes_a_queued_key_transfer() {
    use crate::host::adapters::outbox::mek_transfer::SUPERSEDED;
    use rekindle_outbox::AttemptOutcome;

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    let joiner = queued_for_a_joiner(&host, &owner).await;
    assert!(host.governance().install_transferred_mek(
        "c1",
        KeyScope::Community,
        &Mek::from_bytes([8; 32], 4)
    ));
    let mut queued = rows(&host).await;
    for _ in 0..100 {
        if queued.len() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        queued = rows(&host).await;
    }
    let ids: Vec<&str> = queued.iter().map(|r| r.client_id.as_str()).collect();
    assert_eq!(
        ids,
        [
            format!("MekTransfer:c1:{joiner}:3"),
            format!("MekTransfer:c1:{joiner}:4")
        ],
        "the rotation queued the joiner's key at generation 4"
    );
    assert!(
        matches!(attempt(&host, &queued[0]).await, AttemptOutcome::Superseded(r) if r == SUPERSEDED),
        "the generation-3 row settles superseded"
    );
    assert!(
        !matches!(
            attempt(&host, &queued[1]).await,
            AttemptOutcome::Superseded(_)
        ),
        "the generation-4 row is attempted"
    );
    host.shutdown().await.unwrap();
}

/// Review S7m part 1 F3 (the kick race): a departure (the kick, ban and
/// leave paths all end in `forget_member`) has discarded the member's row
/// when it returns, and a later bootstrap of the member queues nothing.
#[tokio::test]
async fn a_departure_discards_the_member_s_key_transfer_before_it_returns() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    let joiner = queued_for_a_joiner(&host, &owner).await;
    merge_removal(&host, &joiner);
    host.governance().forget_member("c1", &joiner).await;
    assert!(
        rows(&host).await.is_empty(),
        "the kicked member's row is gone"
    );
    deliver(&host, &joiner).await;
    assert!(
        rows(&host).await.is_empty(),
        "no key is queued for a kicked member"
    );
    host.shutdown().await.unwrap();
}

/// Review S7m part 1 F5: our leave has discarded every row of the
/// community when its local teardown returns, and nothing queues after.
#[tokio::test]
async fn a_leave_discards_every_key_transfer_of_the_community() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    let joiner = queued_for_a_joiner(&host, &owner).await;
    host.governance()
        .forget_left_community("c1")
        .await
        .expect("forgotten");
    assert!(rows(&host).await.is_empty(), "the leave discards the rows");
    deliver(&host, &joiner).await;
    assert!(
        rows(&host).await.is_empty(),
        "no key is queued after the leave"
    );
    host.shutdown().await.unwrap();
}

/// Review S7m part 1 F5: a member the merge removed before its row's
/// attempt (its departure not yet forgotten here) is sent nothing: the
/// row settles unsent, and its bootstrap queues nothing more.
#[tokio::test]
async fn a_non_reader_s_key_transfer_settles_unsent() {
    use crate::host::adapters::outbox::mek_transfer::DEPARTED;
    use rekindle_outbox::AttemptOutcome;

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    let joiner = queued_for_a_joiner(&host, &owner).await;
    merge_removal(&host, &joiner);
    let queued = rows(&host).await;
    assert!(
        matches!(attempt(&host, &queued[0]).await, AttemptOutcome::Superseded(r) if r == DEPARTED),
        "a non-reader's row settles unsent"
    );
    deliver(&host, &joiner).await;
    assert_eq!(
        rows(&host).await.len(),
        1,
        "nothing more is queued for a non-reader"
    );
    host.shutdown().await.unwrap();
}
