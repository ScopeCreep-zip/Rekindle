//! The creator-set round's frames through the Host (plan E3.2 S7s.3,
//! decisions (16), (20)): a friend's invite is held and shown, its
//! withdrawal takes it; a co-creator's signed accept is noted only for a
//! creation of ours that waits on it, a bad signature never.

use rekindle_crypto::Identity;
use rekindle_transport::operations::direct::receive;
use rekindle_transport::operations::dm::handshake::client_prefix;
use rekindle_transport::operations::friend::creator_round::{self, CreatorFrame};
use rekindle_types::id::PseudonymKey;

use super::adapters::friends::HostFriends;
use super::direct_tests::ann;
use super::game_totals_tests::{datagram, open_session};
use super::receive_tests::{drain, mentions};
use super::Host;

fn anchor() -> String {
    format!("VLD0:{}", "Q".repeat(43))
}

/// `owner`'s newest outbox row of a creator-set kind, as a datagram.
async fn last_frame(
    host: &Host,
    owner: [u8; 32],
    kind: rekindle_codec::capnp_codec::direct::DmTypeId,
) -> Vec<u8> {
    let o = hex::encode(owner);
    let prefix = client_prefix(kind);
    let rows = host
        .ctx
        .db
        .current()
        .unwrap()
        .call(move |c| rekindle_db::repo::outbox::prefix_rows(c, &o, ""))
        .await
        .unwrap();
    let row = rows
        .into_iter()
        .rfind(|row| row.client_id.starts_with(&prefix))
        .expect("a frame of the kind");
    datagram(&row)
}

async fn held(host: &Host, owner: [u8; 32]) -> usize {
    let o = hex::encode(owner);
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |c| rekindle_db::repo::creator_round::invites(c, &o))
        .await
        .unwrap()
        .len()
}

/// Decision (20): Bob holds Ann's invite (it survives a restart in
/// `creator_invites`) and sees it; Ann's withdrawal takes it.
#[tokio::test]
async fn an_invite_is_held_and_its_withdrawal_takes_it() {
    use rekindle_codec::capnp_codec::direct::DmTypeId;
    let (dir, theirs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (host, owner) = ann(&dir).await;
    let (other, bob) = ann(&theirs).await;
    open_session(&host, owner, &other, bob).await;
    let ann_friends = HostFriends::of(&host.ctx);
    let invite = CreatorFrame::Invite {
        anchor: anchor(),
        community_name: "LAN Party".into(),
    };
    creator_round::send(&ann_friends, &hex::encode(bob), &invite)
        .await
        .unwrap();
    let mut events = other.ctx.journal.subscribe(None);
    let bob_friends = HostFriends::of(&other.ctx);
    receive(
        &bob_friends,
        &last_frame(&host, owner, DmTypeId::CreatorInvite).await,
    )
    .await;
    assert_eq!(mentions(&drain(&mut events).await, "CreatorInvited"), 1);
    assert_eq!(held(&other, bob).await, 1);

    let withdrawn = CreatorFrame::Withdrawn { anchor: anchor() };
    creator_round::send(&ann_friends, &hex::encode(bob), &withdrawn)
        .await
        .unwrap();
    receive(
        &bob_friends,
        &last_frame(&host, owner, DmTypeId::CreatorWithdrawn).await,
    )
    .await;
    assert_eq!(
        mentions(&drain(&mut events).await, "CreatorInviteWithdrawn"),
        1
    );
    assert_eq!(held(&other, bob).await, 0);
}

/// Decision (16): Ann notes Bob's accept only while her creation waits on
/// him, and only with a valid pseudonym signature.
#[tokio::test]
async fn an_accept_counts_only_for_a_waiting_creation_and_a_valid_signature() {
    use rekindle_codec::capnp_codec::direct::DmTypeId;
    let (dir, theirs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (host, owner) = ann(&dir).await;
    let (other, bob) = ann(&theirs).await;
    open_session(&host, owner, &other, bob).await;
    let (o, b) = (hex::encode(owner), hex::encode(bob));
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |c| rekindle_db::repo::creator_round::put_pending(c, &o, &anchor(), &[b]))
        .await
        .unwrap();
    let pseudonym = Identity::generate();
    let key = PseudonymKey(pseudonym.public_key_bytes());
    let good = pseudonym
        .sign(&rekindle_governance::genesis::creator_accept_signing_bytes(
            &anchor(),
            &key,
        ))
        .to_bytes();
    let bob_friends = HostFriends::of(&other.ctx);
    let ann_friends = HostFriends::of(&host.ctx);
    let mut events = host.ctx.journal.subscribe(None);
    for signature in [[0u8; 64], good, good] {
        let accept = CreatorFrame::Accept {
            anchor: anchor(),
            pseudonym: key.0,
            signature,
        };
        creator_round::send(&bob_friends, &hex::encode(owner), &accept)
            .await
            .unwrap();
        receive(
            &ann_friends,
            &last_frame(&other, bob, DmTypeId::CreatorAccept).await,
        )
        .await;
    }
    // The bad signature and the repeat (no longer waited on) are refused.
    assert_eq!(mentions(&drain(&mut events).await, "accepted"), 1);
    let answers = host.ctx.friend_stores.creator_answers.lock();
    assert_eq!(
        answers[&anchor()][&hex::encode(bob)],
        Some((key.0, good)),
        "the verified accept is held for the finish"
    );
}
