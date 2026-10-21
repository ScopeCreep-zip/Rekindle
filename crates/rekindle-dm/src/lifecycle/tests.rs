//! Three members (Ann creates, Bea and Cy join) over in-memory hosts, the
//! frames carried by hand.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;
use rekindle_secrets::ed25519_dalek::{Signer as _, SigningKey};
use rekindle_utils::{OsRandom, RandomSource};

use super::*;
use crate::group::tests::{member_of, signer};

struct Host {
    key: SigningKey,
    groups: Mutex<HashMap<GroupId, GroupState>>,
    keys: Mutex<HashMap<(GroupId, String), GroupSenderKey>>,
    invites: Mutex<HashMap<GroupId, PendingInvite>>,
    events: Mutex<Vec<GroupDmEvent>>,
}

impl Host {
    fn new(n: u8) -> Self {
        Self {
            key: signer(n),
            groups: Mutex::default(),
            keys: Mutex::default(),
            invites: Mutex::default(),
            events: Mutex::default(),
        }
    }
    fn id(&self) -> String {
        member_of(&self.key).identity
    }
}

#[async_trait]
impl GroupDmDeps for Host {
    fn own_identity(&self) -> String {
        self.id()
    }
    fn now_ms(&self) -> u64 {
        1
    }
    fn rng(&self) -> &dyn RandomSource {
        &OsRandom
    }
    fn sign_identity(&self, message: &[u8]) -> [u8; 64] {
        self.key.sign(message).to_bytes()
    }
    async fn display_name(&self, identity: &str) -> String {
        identity[..8].to_owned()
    }
    async fn group(&self, group_id: &GroupId) -> Result<Option<GroupState>, DmError> {
        Ok(self.groups.lock().get(group_id).cloned())
    }
    async fn groups(&self) -> Result<Vec<GroupState>, DmError> {
        Ok(self.groups.lock().values().cloned().collect())
    }
    async fn insert_group(&self, group: &GroupState) -> Result<(), DmError> {
        self.groups.lock().insert(group.group_id, group.clone());
        Ok(())
    }
    async fn update_group(&self, id: &GroupId, roster: &Roster, epoch: u32) -> Result<(), DmError> {
        let mut groups = self.groups.lock();
        let g = groups
            .get_mut(id)
            .ok_or_else(|| DmError::storage("no group"))?;
        g.roster = roster.clone();
        g.epoch = epoch;
        Ok(())
    }
    async fn delete_group(&self, id: &GroupId) -> Result<(), DmError> {
        self.groups.lock().remove(id);
        self.keys.lock().retain(|(g, _), _| g != id);
        Ok(())
    }
    async fn put_sender_key(
        &self,
        id: &GroupId,
        m: &str,
        key: &GroupSenderKey,
    ) -> Result<(), DmError> {
        self.keys.lock().insert((*id, m.to_owned()), key.clone());
        Ok(())
    }
    async fn latest_sender_key(
        &self,
        id: &GroupId,
        m: &str,
    ) -> Result<Option<GroupSenderKey>, DmError> {
        Ok(self.keys.lock().get(&(*id, m.to_owned())).cloned())
    }
    async fn delete_member_keys(&self, id: &GroupId, m: &str) -> Result<(), DmError> {
        self.keys.lock().remove(&(*id, m.to_owned()));
        Ok(())
    }
    async fn stash_invite(&self, invite: &PendingInvite) -> Result<(), DmError> {
        self.invites
            .lock()
            .insert(invite.invite.group_id, invite.clone());
        Ok(())
    }
    async fn take_invite(&self, id: &GroupId) -> Result<Option<PendingInvite>, DmError> {
        Ok(self.invites.lock().remove(id))
    }
    fn publish(&self, event: GroupDmEvent) {
        self.events.lock().push(event);
    }
}

/// Deliver `out` from `from` to each named host's `on_member_key`, and
/// every reply back, until the keys settle (a member who has not accepted
/// yet holds no group: admission drops the frame).
async fn deliver_keys(hosts: &[&Host], from: &Host, out: Outbound<GroupDmAccept>) {
    let mut queue = vec![(from.id(), out)];
    let mut rounds = 0;
    while let Some((sender, out)) = queue.pop() {
        rounds += 1;
        assert!(rounds < 16, "the key exchange settles");
        for to in &out.to {
            let host = hosts.iter().find(|h| h.id() == *to).expect("a host");
            if host.groups.lock().get(&out.frame.group_id).is_none() {
                continue;
            }
            if let Some(reply) = on_member_key(*host, &sender, &out.frame)
                .await
                .expect("key")
            {
                queue.push((host.id(), reply));
            }
        }
    }
}

async fn three() -> (Host, Host, Host, GroupId) {
    let (ann, bea, cy) = (Host::new(1), Host::new(2), Host::new(3));
    let created = create(
        &ann,
        "squad",
        member_of(&ann.key),
        vec![member_of(&bea.key), member_of(&cy.key)],
    )
    .await
    .expect("create");
    let gid = created.group.group_id;
    assert_eq!(created.invite.to, vec![bea.id(), cy.id()]);
    for h in [&bea, &cy] {
        on_invite(h, &ann.id(), created.invite.frame.clone())
            .await
            .expect("invite");
    }
    // Cy accepts first, Bea second: both still hold each other's key.
    for h in [&cy, &bea] {
        let out = accept(h, &gid).await.expect("accept");
        deliver_keys(&[&ann, &bea, &cy], h, out).await;
    }
    (ann, bea, cy, gid)
}

#[tokio::test]
async fn members_exchange_keys_and_read_each_other() {
    let (ann, bea, cy, gid) = three().await;
    for (sender, readers) in [
        (&ann, [&bea, &cy]),
        (&bea, [&ann, &cy]),
        (&cy, [&ann, &bea]),
    ] {
        let content = seal_for_send(sender, &gid, b"hi").await.expect("seal");
        for r in readers {
            assert_eq!(
                open_received(r, &sender.id(), &content)
                    .await
                    .expect("open"),
                b"hi"
            );
        }
        // Another member's claim to the entry does not open.
        assert!(open_received(readers[0], &readers[1].id(), &content)
            .await
            .is_err());
    }
    assert!(matches!(
        bea.events.lock().first(),
        Some(GroupDmEvent::Invited { members, .. }) if members.len() == 3
    ));
}

#[tokio::test]
async fn a_leaver_reads_nothing_after_the_rotation() {
    let (ann, bea, cy, gid) = three().await;
    let old_ann = seal_for_send(&ann, &gid, b"before").await.expect("seal");
    let left = leave(&cy, &gid).await.expect("leave");
    assert!(cy.groups.lock().is_empty());
    for h in [&ann, &bea] {
        let out = on_member_left(h, &cy.id(), &left.frame)
            .await
            .expect("left");
        assert_eq!(out.frame.epoch, 1);
        assert!(!out.to.contains(&cy.id()));
        deliver_keys(&[&ann, &bea], h, out).await;
    }
    let new_ann = seal_for_send(&ann, &gid, b"after").await.expect("seal");
    assert_eq!(new_ann.epoch, 1);
    assert_eq!(
        open_received(&bea, &ann.id(), &new_ann)
            .await
            .expect("open"),
        b"after"
    );
    // An entry at Ann's old epoch is refused now that Bea holds epoch 1.
    assert!(matches!(
        open_received(&bea, &ann.id(), &old_ann).await,
        Err(DmError::StaleEpoch { got: 0, current: 1 })
    ));
    // Cy holds no group, and Cy's frames are no longer Ann's members'.
    assert!(open_received(&cy, &ann.id(), &new_ann).await.is_err());
    assert!(matches!(
        open_received(&ann, &cy.id(), &new_ann).await,
        Err(DmError::NotOnRoster(_))
    ));
}

#[tokio::test]
async fn a_newer_epoch_waits_for_its_key_and_stale_keys_are_refused() {
    let (ann, bea, _cy, gid) = three().await;
    let rotated = rotate_own_key(&ann, &gid).await.expect("rotate");
    let ahead = seal_for_send(&ann, &gid, b"x").await.expect("seal");
    assert!(matches!(
        open_received(&bea, &ann.id(), &ahead).await,
        Err(DmError::KeyMissing { epoch: 1, .. })
    ));
    assert_eq!(
        on_member_key(&bea, &ann.id(), &rotated.frame)
            .await
            .expect("key"),
        None
    );
    assert_eq!(
        open_received(&bea, &ann.id(), &ahead).await.expect("open"),
        b"x"
    );
    let mut stale = rotated.frame.clone();
    stale.epoch = 0;
    assert!(matches!(
        on_member_key(&bea, &ann.id(), &stale).await,
        Err(DmError::StaleEpoch { .. })
    ));
}

#[tokio::test]
async fn invites_are_refused_when_forged_or_not_ours_and_decline_rotates() {
    let (ann, bea, dee) = (Host::new(1), Host::new(2), Host::new(4));
    let cy = Host::new(3);
    let created = create(
        &ann,
        "squad",
        member_of(&ann.key),
        vec![member_of(&bea.key), member_of(&cy.key)],
    )
    .await
    .expect("create");
    let gid = created.group.group_id;
    assert!(matches!(
        on_invite(&dee, &ann.id(), created.invite.frame.clone()).await,
        Err(DmError::NotOnRoster(_))
    ));
    assert!(matches!(
        on_invite(&bea, &cy.id(), created.invite.frame.clone()).await,
        Err(DmError::NotOnRoster(_))
    ));
    on_invite(&bea, &ann.id(), created.invite.frame)
        .await
        .expect("invite");
    let declined = decline(&bea, &gid).await.expect("decline");
    assert_eq!(declined.to, vec![ann.id()]);
    assert!(accept(&bea, &gid).await.is_err());
    let out = on_decline(&ann, &bea.id(), &declined.frame)
        .await
        .expect("decline");
    assert_eq!((out.frame.epoch, out.to), (1, vec![cy.id()]));
    assert!(create(&ann, "", member_of(&ann.key), vec![]).await.is_err());
}
