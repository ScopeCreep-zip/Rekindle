//! A community's info edit and its invites on the Host (plan E3.1 D1),
//! offline: what each refuses before it writes, and the invite list's order
//! and our counts.

use rekindle_governance::state::{GovernanceState, InviteState};
use rekindle_types::id::PseudonymKey;

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::governance::InfoEdit;

fn invite(code_hash: &str, creator: u8) -> InviteState {
    InviteState {
        code_hash: code_hash.into(),
        max_uses: 0,
        expires_at: None,
        secrets_record_key: String::new(),
        created: rekindle_governance::fixture::stamp(1),
        creator_pseudonym: PseudonymKey([creator; 32]),
        uses: 0,
    }
}

/// An edit of a community whose governance is not merged writes nothing:
/// it has no metadata to keep the unchanged fields from.
#[tokio::test]
async fn an_info_edit_needs_the_merged_governance() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let edit = InfoEdit {
        name: Some("Renamed".into()),
        ..InfoEdit::default()
    };
    let refused = host.governance().update_info("c1", edit).await.unwrap_err();
    assert!(refused.contains("governance"), "{refused}");
    let name = host.ctx.communities.map().read()["c1"].name.clone();
    assert_ne!(name, "Renamed");
    host.shutdown().await.unwrap();
}

/// A revoke names a live invite by its id; an id merged governance does
/// not hold is refused before any write.
#[tokio::test]
async fn a_revoke_names_a_live_invite() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    host.ctx
        .communities
        .map()
        .write()
        .get_mut("c1")
        .unwrap()
        .governance_state = Some(GovernanceState::default());
    let refused = host
        .governance()
        .revoke_invite("c1", [9; 16])
        .await
        .unwrap_err();
    assert!(refused.contains("no live invite"), "{refused}");
    host.shutdown().await.unwrap();
}

/// A community whose id is a record key, as every real one is, with the
/// merged governance `gov`.
const KEYED_ANCHOR: &str = "VLD0:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA";

async fn seed_keyed_community(host: &crate::host::Host, owner: &str, gov: GovernanceState) {
    host.ctx.communities.map().write().insert(
        KEYED_ANCHOR.into(),
        crate::host::community::CommunityState {
            member_registry_key: Some("registry".into()),
            governance_state: Some(gov),
            ..crate::host::community::CommunityState::new(KEYED_ANCHOR)
        },
    );
    let owner = owner.to_owned();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            rekindle_db::repo::communities::insert_if_absent(
                conn,
                &owner,
                &rekindle_db::repo::communities::NewCommunity {
                    id: KEYED_ANCHOR,
                    name: "C",
                    joined_at: 1,
                    my_pseudonym_key: "",
                    mek_generation: 0,
                    member_registry_key: Some("registry"),
                    my_subkey_index: None,
                },
            )?;
            rekindle_db::repo::community_invites::insert(
                conn,
                &owner,
                KEYED_ANCHOR,
                (OWN_CODE, "ours", OWN_URL),
                7,
            )
        })
        .await
        .unwrap();
}

/// Our stored code of the invite `ours`: 32 hex, so its link is built.
const OWN_CODE: &str = "3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c";

/// The link as minted for `ours` (plan IV1b): anchor, secrets record, code, the
/// `InviteCreated` op by hash, its chain seq, and the inviter's frontier seq.
const OWN_URL: &str = "rekindle://invite/VLD0:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA/VLD0:CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCA/3f1a9c0b7e2d4f6a8b1c3e5d7f9a0b2c/5e0f9c2b7a1d3e4f6a8b0c2d4e6f8a0b1c3d5e7f9a2b4c6d8e0f1a3b5c7d9e2f/7/3";

/// The list is merged governance in invite-id order, each invite with the
/// use count the merge gates (`InviteState.uses`, plan E3.2 S8: the invite
/// gate counts the joins that redeemed it), and our link and mint time on
/// the invite we minted.
#[tokio::test]
async fn the_invite_list_is_in_id_order_with_our_counts() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    let mut gov = GovernanceState::default();
    let mut theirs = invite("theirs", 2);
    theirs.uses = 2;
    let mut ours = invite("ours", 1);
    ours.uses = 3;
    gov.invites.insert([2; 16], theirs);
    gov.invites.insert([1; 16], ours);
    seed_keyed_community(&host, &owner, gov).await;
    let list = host.governance().list_invites(KEYED_ANCHOR).await.unwrap();
    let ids: Vec<_> = list.iter().map(|i| i.invite_id.clone()).collect();
    assert_eq!(ids, vec!["01".repeat(16), "02".repeat(16)]);
    assert_eq!((list[0].uses, list[0].created_at), (3, Some(7)));
    assert_eq!((list[1].uses, list[1].created_at), (2, None));
    assert_eq!(list[1].created_by, "02".repeat(32));
    host.shutdown().await.unwrap();
}

/// The inviter's list shows, beside each live invite, the fingerprint the
/// joiner's review names for that invite's link (plan E7.15), so the two
/// screens can be compared out of band like a safety number.
#[tokio::test]
async fn the_invite_list_shows_the_fingerprint_the_joiners_review_names() {
    use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, IpcResponse, LifecycleRequest};
    use rekindle_ipc::{Caller, HostApi};
    use rekindle_types::intent::Subject;

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    let mut gov = GovernanceState::default();
    let mut ours = invite("ours", 1);
    ours.secrets_record_key = "VLD0:CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCA".into();
    gov.invites.insert([1; 16], ours);
    seed_keyed_community(&host, &owner, gov).await;
    let list = host.governance().list_invites(KEYED_ANCHOR).await.unwrap();
    let url = list[0].url.clone().expect("our invite carries its link");
    let join = IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityJoin { invite: url },
    ));
    let review = match host.call(Caller::shell(), join).await {
        IpcResponse::IntentRequired(review) => review,
        other => panic!("a join link is held for review, not {other:?}"),
    };
    assert_eq!(
        review.subject,
        Subject::Named {
            name: list[0].fingerprint.clone()
        }
    );
    host.shutdown().await.unwrap();
}
