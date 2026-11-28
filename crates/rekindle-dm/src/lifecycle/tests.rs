//! Three members (Ann creates, Bea and Cy join) over in-memory hosts, the
//! frames carried by hand.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;
use rekindle_secrets::ed25519_dalek::{Signer as _, SigningKey};
use rekindle_utils::{OsRandom, RandomSource};

use super::*;
use crate::group::tests::{member_of, signer};
use crate::ports::RendezvousSlot;

struct Host {
    key: SigningKey,
    membership_gate: tokio::sync::Mutex<()>,
    groups: Mutex<HashMap<GroupId, GroupState>>,
    keys: Mutex<HashMap<(GroupId, String), GroupSenderKey>>,
    invites: Mutex<HashMap<GroupId, PendingInvite>>,
    chains: Mutex<HashMap<(GroupId, String), String>>,
    rendezvous: Mutex<HashMap<GroupId, RendezvousSlot>>,
    events: Mutex<Vec<GroupDmEvent>>,
}

/// The creator's minted rendezvous record (decision 67 item 3).
fn mint() -> RendezvousMint {
    RendezvousMint {
        record_key: "VLD0:rendezvous".into(),
        writers: (0..8).map(|i| format!("VLD0:writer-{i}")).collect(),
    }
}

/// The frame `outs` sends to `who`, which is sent to it alone.
fn frame_for<F: Clone>(outs: &[Outbound<F>], who: &str) -> F {
    let out = outs
        .iter()
        .find(|o| o.to == [who.to_owned()])
        .expect("one frame to this member");
    out.frame.clone()
}

/// Every member `outs` goes to, one frame each.
fn recipients<F>(outs: &[Outbound<F>]) -> Vec<String> {
    outs.iter()
        .map(|o| {
            assert_eq!(o.to.len(), 1, "one recipient per frame");
            o.to[0].clone()
        })
        .collect()
}

async fn grow_with(
    h: &Host,
    gid: &GroupId,
    added: Vec<rekindle_types::group_dm::GroupDmRosterMember>,
) -> Result<Grown, DmError> {
    grow(h, gid, added, &mint()).await
}

impl Host {
    fn new(n: u8) -> Self {
        Self {
            key: signer(n),
            membership_gate: tokio::sync::Mutex::default(),
            groups: Mutex::default(),
            keys: Mutex::default(),
            invites: Mutex::default(),
            chains: Mutex::default(),
            rendezvous: Mutex::default(),
            events: Mutex::default(),
        }
    }
    fn id(&self) -> String {
        member_of(&self.key).identity
    }
    fn chain(&self) -> String {
        format!("VLD0:chain-{}", &self.id()[..8])
    }
}

#[async_trait]
impl GroupDmDeps for Host {
    fn membership_gate(&self) -> &tokio::sync::Mutex<()> {
        &self.membership_gate
    }

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
    async fn put_chain(&self, id: &GroupId, m: &str, key: &str) -> Result<(), DmError> {
        self.chains
            .lock()
            .insert((*id, m.to_owned()), key.to_owned());
        Ok(())
    }
    async fn chain(&self, id: &GroupId, m: &str) -> Result<Option<String>, DmError> {
        Ok(self.chains.lock().get(&(*id, m.to_owned())).cloned())
    }
    async fn chains(&self, id: &GroupId) -> Result<Vec<(String, String)>, DmError> {
        let held = self.chains.lock();
        Ok(held
            .iter()
            .filter(|((g, _), _)| g == id)
            .map(|((_, m), k)| (m.clone(), k.clone()))
            .collect())
    }
    async fn delete_member_keys(&self, id: &GroupId, m: &str) -> Result<(), DmError> {
        self.keys.lock().remove(&(*id, m.to_owned()));
        self.chains.lock().remove(&(*id, m.to_owned()));
        Ok(())
    }
    async fn stash_invite(&self, invite: &PendingInvite) -> Result<(), DmError> {
        self.invites
            .lock()
            .insert(invite.invite.group_id, invite.clone());
        Ok(())
    }
    async fn invites(&self) -> Result<Vec<PendingInvite>, DmError> {
        Ok(self.invites.lock().values().cloned().collect())
    }
    async fn take_invite(&self, id: &GroupId) -> Result<Option<PendingInvite>, DmError> {
        Ok(self.invites.lock().remove(id))
    }
    async fn put_rendezvous(&self, id: &GroupId, slot: &RendezvousSlot) -> Result<(), DmError> {
        self.rendezvous.lock().insert(*id, slot.clone());
        Ok(())
    }
    async fn rendezvous(&self, id: &GroupId) -> Result<Option<RendezvousSlot>, DmError> {
        Ok(self.rendezvous.lock().get(id).cloned())
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
        group::new_group_id(&OsRandom),
        "squad",
        member_of(&ann.key),
        vec![member_of(&bea.key), member_of(&cy.key)],
        &ann.chain(),
        &mint(),
    )
    .await
    .expect("create");
    let gid = created.group.group_id;
    assert_eq!(recipients(&created.invites), vec![bea.id(), cy.id()]);
    for h in [&bea, &cy] {
        on_invite(h, &ann.id(), frame_for(&created.invites, &h.id()))
            .await
            .expect("invite");
    }
    // Cy accepts first, Bea second: both still hold each other's key.
    for h in [&cy, &bea] {
        let out = accept(h, &gid, &h.chain()).await.expect("accept");
        deliver_keys(&[&ann, &bea, &cy], h, out).await;
    }
    (ann, bea, cy, gid)
}

#[path = "grow_tests.rs"]
mod grow_tests;

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
            .expect("left")
            .expect("matching generation");
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
        group::new_group_id(&OsRandom),
        "squad",
        member_of(&ann.key),
        vec![member_of(&bea.key), member_of(&cy.key)],
        &ann.chain(),
        &mint(),
    )
    .await
    .expect("create");
    let gid = created.group.group_id;
    assert!(matches!(
        on_invite(&dee, &ann.id(), frame_for(&created.invites, &bea.id())).await,
        Err(DmError::NotOnRoster(_))
    ));
    assert!(matches!(
        on_invite(&bea, &cy.id(), frame_for(&created.invites, &bea.id())).await,
        Err(DmError::NotOnRoster(_))
    ));
    on_invite(&bea, &ann.id(), frame_for(&created.invites, &bea.id()))
        .await
        .expect("invite");
    let declined = decline(&bea, &gid).await.expect("decline");
    assert_eq!(declined.to, vec![ann.id()]);
    assert!(accept(&bea, &gid, &bea.chain()).await.is_err());
    let out = on_decline(&ann, &bea.id(), &declined.frame)
        .await
        .expect("decline")
        .expect("matching generation");
    assert_eq!((out.frame.epoch, out.to), (1, vec![cy.id()]));
    assert!(create(
        &ann,
        [1; 16],
        "",
        member_of(&ann.key),
        vec![],
        &ann.chain(),
        &mint()
    )
    .await
    .is_err());
}

#[tokio::test]
async fn each_member_holds_every_chain_as_its_member_announced_it() {
    let (ann, bea, cy, gid) = three().await;
    for h in [&ann, &bea, &cy] {
        let mut held = h.chains(&gid).await.expect("chains");
        held.sort();
        let mut want: Vec<_> = [&ann, &bea, &cy]
            .iter()
            .map(|m| (m.id(), m.chain()))
            .collect();
        want.sort();
        assert_eq!(held, want);
    }
    // Bea replaces her chain; a non-member's or our own announcement is refused.
    on_member_chain(&ann, &bea.id(), &gid, "VLD0:chain-new")
        .await
        .expect("chain");
    assert_eq!(
        GroupDmDeps::chain(&ann, &gid, &bea.id())
            .await
            .unwrap()
            .as_deref(),
        Some("VLD0:chain-new")
    );
    assert!(on_member_chain(&ann, &Host::new(9).id(), &gid, "VLD0:x")
        .await
        .is_err());
    assert!(on_member_chain(&ann, &ann.id(), &gid, "VLD0:x")
        .await
        .is_err());
    // A rotation carries our chain.
    let out = rotate_own_key(&bea, &gid).await.expect("rotate");
    assert_eq!(out.frame.chain_record_key, bea.chain());
}

async fn created_three(ann: &Host, bea: &Host, cy: &Host) -> Created {
    create(
        ann,
        group::new_group_id(&OsRandom),
        "squad",
        member_of(&ann.key),
        vec![member_of(&bea.key), member_of(&cy.key)],
        &ann.chain(),
        &mint(),
    )
    .await
    .expect("create")
}

/// Decision 67 item 3: one invite per member, each with its own slot and
/// writer under one signature; the creator and each accepter hold their
/// place and the signed plan.
#[tokio::test]
async fn each_member_gets_its_own_rendezvous_slot_and_writer() {
    let (ann, bea, cy) = (Host::new(1), Host::new(2), Host::new(3));
    let created = created_three(&ann, &bea, &cy).await;
    let gid = created.group.group_id;
    let to_bea = frame_for(&created.invites, &bea.id());
    let to_cy = frame_for(&created.invites, &cy.id());
    assert_eq!((to_bea.slot, to_cy.slot), (1, 2));
    assert_eq!(to_bea.slot_writer, "VLD0:writer-1");
    assert_eq!(to_cy.slot_writer, "VLD0:writer-2");
    assert_eq!(to_bea.signing_bytes(), to_cy.signing_bytes());
    let own = ann.rendezvous.lock()[&gid].clone();
    assert_eq!((own.slot, own.writer.as_str()), (0, "VLD0:writer-0"));
    for (h, frame) in [(&bea, to_bea), (&cy, to_cy)] {
        on_invite(h, &ann.id(), frame).await.expect("invite");
        accept(h, &gid, &h.chain()).await.expect("accept");
    }
    let held = bea.rendezvous.lock()[&gid].clone();
    assert_eq!(
        (held.record_key.as_str(), held.slot, held.writer.as_str()),
        ("VLD0:rendezvous", 1, "VLD0:writer-1")
    );
    assert_eq!(held.member_at(0), Some(ann.id().as_str()));
    assert_eq!(held.member_at(2), Some(cy.id().as_str()));
    assert_eq!(cy.rendezvous.lock()[&gid].slot, 2);
}

/// An invite carrying another member's slot (Cy's row relabelled onto
/// Bea's frame) still verifies, since the slot is unsigned, but accept
/// refuses it against the signed plan and stores nothing.
#[tokio::test]
async fn an_invite_with_another_members_slot_is_refused_at_accept() {
    let (ann, bea, cy) = (Host::new(1), Host::new(2), Host::new(3));
    let created = created_three(&ann, &bea, &cy).await;
    let gid = created.group.group_id;
    let mut frame = frame_for(&created.invites, &bea.id());
    frame.slot = 2;
    frame.slot_writer = "VLD0:writer-2".into();
    on_invite(&bea, &ann.id(), frame).await.expect("verifies");
    assert!(matches!(
        accept(&bea, &gid, &bea.chain()).await,
        Err(DmError::InvalidRoster(_))
    ));
    assert!(bea.rendezvous.lock().is_empty());
    assert!(bea.groups.lock().is_empty());
}
