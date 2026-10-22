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
        created_lamport: 1,
        creator_pseudonym: PseudonymKey([creator; 32]),
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

/// The list is merged governance in invite-id order, with our use count
/// on the invite we minted and none on another member's.
#[tokio::test]
async fn the_invite_list_is_in_id_order_with_our_counts() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let mut gov = GovernanceState::default();
    gov.invites.insert([2; 16], invite("theirs", 2));
    gov.invites.insert([1; 16], invite("ours", 1));
    host.ctx
        .communities
        .map()
        .write()
        .get_mut("c1")
        .unwrap()
        .governance_state = Some(gov);
    let o = owner.clone();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            rekindle_db::repo::community_invites::insert(conn, &o, "c1", "CODE", "ours", 7)?;
            rekindle_db::repo::community_invites::bump_uses(conn, &o, "c1", "ours").map(drop)
        })
        .await
        .unwrap();
    let list = host.governance().list_invites("c1").await.unwrap();
    let ids: Vec<_> = list.iter().map(|i| i.invite_id.clone()).collect();
    assert_eq!(ids, vec!["01".repeat(16), "02".repeat(16)]);
    assert_eq!((list[0].uses, list[0].created_at), (1, Some(7)));
    assert_eq!((list[1].uses, list[1].created_at), (0, None));
    assert_eq!(list[1].created_by, "02".repeat(32));
    host.shutdown().await.unwrap();
}
