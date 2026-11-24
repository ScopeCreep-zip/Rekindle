//! The unlock order of the community streams (finding JW2): the outbox
//! starts at unlock step 1 and its run loop's first pass can come before
//! step 3e restores the communities. A `bootstrap` row the last lock left
//! queued, attempted then, must not read the empty cache as a community
//! left: it defers until `OutboxSlot::hand_over_communities` (step 3e''')
//! installs the sender, and is attempted with its community held after.
//! The order is forced here, not raced: the slot is started on a closed
//! scope (no run loop), and each pass is one explicit `pump`.

use rekindle_lifecycle::SessionScope;
use rekindle_outbox::{Deadline, Entry, Outbox, OutboxDeps as _, StreamKind};
use rekindle_types::message::SendState;

use super::{HostOutbox, OutboxSlot};
use crate::host::test_fixtures::{create_ann, offline_host, seed_registry_community};

const HASH: [u8; 32] = [9; 32];
const KEY_ROW: &str = "mek_transfer:c1:bb:junk";

async fn row(outbox: &Outbox<HostOutbox>, id: &str) -> rekindle_outbox::OutboxRow {
    outbox.deps().row(id).await.unwrap().unwrap()
}

#[tokio::test]
async fn a_bootstrap_row_attempted_before_the_communities_are_restored_defers_until_the_hand_over()
{
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let ann = create_ann(&host).await;
    // The unlocked Host's own outbox must not attempt the rows queued here.
    host.ctx.outbox.senders.remove(StreamKind::Bootstrap);
    host.ctx.outbox.senders.remove(StreamKind::MekTransfer);

    // Unlock step 1: a fresh outbox, no community restored yet.
    let slot = OutboxSlot::default();
    let closed = SessionScope::closed("jw2 proof");
    assert!(slot.start(&host.ctx, &closed, &ann.public_key).is_err());
    let outbox = slot.current.read().clone().unwrap();
    assert!(host.ctx.communities.map().read().is_empty());

    // The rows the last lock left queued.
    let bootstraps = slot.bootstraps().unwrap();
    bootstraps.queue("c1", "VLD0:inviter", &HASH).await.unwrap();
    let boot_id = format!("bootstrap:c1:{}", hex::encode(HASH));
    let key_entry = Entry {
        kind: StreamKind::MekTransfer,
        scope: "c1:bb".into(),
        client_id: KEY_ROW.into(),
        payload: b"junk".to_vec(),
        deadline: Deadline::Horizon,
    };
    outbox.enqueue(key_entry, None).await.unwrap();

    // The run loop's startup pass, before step 3e.
    outbox.pump().await.unwrap();
    let early = outbox.deps().row(&boot_id).await.unwrap();
    assert_eq!(
        early.map(|r| (r.state, r.wedge_reason)),
        Some((SendState::Queued, None)),
        "a bootstrap row attempted before the restore is kept, not settled as left"
    );
    assert_eq!(row(&outbox, KEY_ROW).await.state, SendState::Queued);

    // Steps 3e (the community restored) and 3e''' (the hand-over).
    seed_registry_community(&host, &ann.public_key).await;
    slot.hand_over_communities(crate::daemon::dispatch::adapter(&host.ctx))
        .await;
    outbox
        .bring_forward(StreamKind::MekTransfer, "c1:bb")
        .await
        .unwrap();
    outbox.pump().await.unwrap();
    let after = row(&outbox, &boot_id).await;
    assert_eq!(
        after.wedge_reason.as_deref(),
        Some("our MemberJoined is not held"),
        "the hand-over makes the row due and its sender reads the community held"
    );
    assert_eq!(
        row(&outbox, KEY_ROW).await.wedge_reason.as_deref(),
        Some("malformed key transfer row"),
        "the key transfer sender is installed by the hand-over"
    );
    host.shutdown().await.unwrap();
}
