//! Roster growth (plan E7.2 G8): only the creator grows the roster, up to
//! eight; every member, old and new, ends up holding every key, and a
//! newcomer reads nothing from before the grow.

use super::*;

/// Ann (the creator) invites Dan into the room of three; Bea and Cy apply
/// the grow and rotate; Dan accepts. The grow's frames.
async fn four() -> (Host, Host, Host, Host, GroupId, Grown) {
    let (ann, bea, cy, gid) = three().await;
    let dan = Host::new(4);
    let grown = grow(&ann, &gid, vec![member_of(&dan.key)])
        .await
        .expect("grow");
    assert_eq!(grown.invite.to, vec![dan.id()]);
    assert_eq!(grown.grow.to, vec![bea.id(), cy.id()]);
    let hosts = [&ann, &bea, &cy, &dan];
    for h in [&bea, &cy] {
        let out = on_roster_grow(h, &ann.id(), &grown.grow.frame)
            .await
            .expect("grow applies")
            .expect("a rotation");
        assert!(!out.to.contains(&dan.id()), "only the members it had");
        deliver_keys(&hosts, h, out).await;
    }
    on_invite(&dan, &ann.id(), grown.invite.frame.clone())
        .await
        .expect("invite");
    let out = accept(&dan, &gid, &dan.chain()).await.expect("accept");
    deliver_keys(&hosts, &dan, out).await;
    (ann, bea, cy, dan, gid, grown)
}

#[tokio::test]
async fn a_grown_room_of_four_reads_each_other() {
    let (ann, bea, cy, dan, gid, grown) = four().await;
    let hosts = [&ann, &bea, &cy, &dan];
    for h in hosts {
        assert_eq!(h.groups.lock()[&gid].roster.members().len(), 4);
    }
    for sender in hosts {
        let content = seal_for_send(sender, &gid, b"hi four").await.expect("seal");
        // The members it had rotated on the grow; Dan's key is its first.
        let epoch = u32::from(sender.id() != dan.id());
        assert_eq!(content.epoch, epoch);
        for r in hosts.iter().filter(|r| r.id() != sender.id()) {
            assert_eq!(
                open_received(*r, &sender.id(), &content)
                    .await
                    .expect("open"),
                b"hi four"
            );
        }
    }
    // The same grow again is a no-op.
    assert_eq!(
        on_roster_grow(&bea, &ann.id(), &grown.grow.frame)
            .await
            .expect("duplicate"),
        None
    );
}

#[tokio::test]
async fn a_newcomer_cannot_open_an_entry_sealed_before_the_grow() {
    let (ann, bea, cy, gid) = three().await;
    let before = seal_for_send(&ann, &gid, b"history").await.expect("seal");
    let dan = Host::new(4);
    let grown = grow(&ann, &gid, vec![member_of(&dan.key)])
        .await
        .expect("grow");
    for h in [&bea, &cy] {
        let out = on_roster_grow(h, &ann.id(), &grown.grow.frame)
            .await
            .expect("grow")
            .expect("rotation");
        deliver_keys(&[&ann, &bea, &cy, &dan], h, out).await;
    }
    on_invite(&dan, &ann.id(), grown.invite.frame.clone())
        .await
        .expect("invite");
    let out = accept(&dan, &gid, &dan.chain()).await.expect("accept");
    deliver_keys(&[&ann, &bea, &cy, &dan], &dan, out).await;
    assert!(matches!(
        open_received(&dan, &ann.id(), &before).await,
        Err(DmError::StaleEpoch { got: 0, current: 1 })
    ));
}

#[tokio::test]
async fn only_the_creator_grows_and_never_past_eight() {
    let (ann, bea, _cy, gid) = three().await;
    let refused = grow(&bea, &gid, vec![member_of(&signer(9))]).await;
    assert!(matches!(refused, Err(DmError::InvalidRoster(r)) if r.contains("creator")));
    let bea_roster = bea.groups.lock()[&gid].roster.clone();
    assert!(group::invite_refused(&bea_roster, &bea.id()).is_some());
    assert!(group::rename_refused(&bea_roster, &bea.id()).is_some());
    let ann_roster = ann.groups.lock()[&gid].roster.clone();
    assert_eq!(group::invite_refused(&ann_roster, &ann.id()), None);
    assert_eq!(group::rename_refused(&ann_roster, &ann.id()), None);
    // A member already on it is refused.
    assert!(matches!(
        grow(&ann, &gid, vec![member_of(&bea.key)]).await,
        Err(DmError::AlreadyMember(_))
    ));
    // Six more would make nine: refused, the roster unchanged.
    let six: Vec<_> = (10..16).map(|n| member_of(&signer(n))).collect();
    assert!(matches!(
        grow(&ann, &gid, six).await,
        Err(DmError::InvalidRoster(_))
    ));
    assert_eq!(ann.groups.lock()[&gid].roster.members().len(), 3);
    let five: Vec<_> = (10..15).map(|n| member_of(&signer(n))).collect();
    grow(&ann, &gid, five).await.expect("eight");
    let full = ann.groups.lock()[&gid].roster.clone();
    assert!(group::invite_refused(&full, &ann.id()).is_some_and(|r| r.contains("full")));
    assert!(matches!(
        grow(&ann, &gid, vec![member_of(&signer(20))]).await,
        Err(DmError::InvalidRoster(r)) if r.contains("full")
    ));
}

#[tokio::test]
async fn a_grow_from_a_non_creator_or_not_extending_is_refused() {
    let (ann, bea, cy, gid) = three().await;
    let dan = Host::new(4);
    let grown = grow(&ann, &gid, vec![member_of(&dan.key)])
        .await
        .expect("grow");
    // Bea relays Ann's grow: Bea is not the creator.
    assert!(on_roster_grow(&cy, &bea.id(), &grown.grow.frame)
        .await
        .is_err());
    assert_eq!(cy.groups.lock()[&gid].roster.members().len(), 3);
}

#[tokio::test]
async fn an_unanswered_invite_is_replaced_by_the_grown_one() {
    let (ann, bea, cy) = (Host::new(1), Host::new(2), Host::new(3));
    let created = create(
        &ann,
        group::new_group_id(&OsRandom),
        "squad",
        member_of(&ann.key),
        vec![member_of(&bea.key), member_of(&cy.key)],
        &ann.chain(),
    )
    .await
    .expect("create");
    let gid = created.group.group_id;
    on_invite(&bea, &ann.id(), created.invite.frame.clone())
        .await
        .expect("invite");
    let dan = Host::new(4);
    let grown = grow(&ann, &gid, vec![member_of(&dan.key)])
        .await
        .expect("grow");
    assert_eq!(
        on_roster_grow(&bea, &ann.id(), &grown.grow.frame)
            .await
            .expect("replaces"),
        None
    );
    assert_eq!(bea.invites.lock()[&gid].invite.roster.len(), 4);
    accept(&bea, &gid, &bea.chain()).await.expect("accept");
    assert_eq!(bea.groups.lock()[&gid].roster.members().len(), 4);
}
