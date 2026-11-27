//! The channel ingest's reader-side VIEW gate (plan E3.2 SEC): a post of
//! a channel an overwrite closes to us is neither stored nor published,
//! and a later grant of the view makes it readable.

use rekindle_channel::error::ChannelError;
use rekindle_governance::overwrite::OverwriteTarget;
use rekindle_governance::state::{GovernanceState, OverwriteState};
use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};
use rekindle_types::permissions::{MANAGE_CHANNELS, MANAGE_ROLES, READ_HISTORY, VIEW_CHANNELS};
use rekindle_types::subscription_events::ChannelMessageEvent;

use super::channel_change_tests::{channel_events, reader, reload, EDIT_CHANNEL};
use super::channel_gap_tests::with_channel;
use super::channel_held_tests::post;
use crate::host::adapters::channel::ingest::Ingested;
use crate::host::adapters::channel::HostChannel;
use crate::host::adapters::governance::view_gain::view_gained;
use crate::host::Host;

/// `c1`'s channel with `@everyone` denied `VIEW_CHANNELS` on it.
fn closed() -> GovernanceState {
    let mut gov = with_channel();
    let channel = ChannelId::from_hex(EDIT_CHANNEL).expect("channel id");
    gov.overwrites.insert(
        (channel, OverwriteTarget::Role(RoleId([0; 16])).key()),
        OverwriteState {
            target_type: "role".into(),
            allow: 0,
            deny: VIEW_CHANNELS,
            stamp: rekindle_governance::fixture::stamp(1),
        },
    );
    gov
}

fn published(events: &[ChannelMessageEvent], id: &str) -> bool {
    events.iter().any(|e| {
        matches!(e, ChannelMessageEvent::New { message_id, body, .. } if message_id == id && body.is_some())
    })
}

async fn stored(host: &Host, owner: &str, body: &str) -> bool {
    reload(host, owner).await.iter().any(|(b, _)| b == body)
}

#[tokio::test]
async fn a_member_denied_view_neither_stores_nor_receives_the_post() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = reader(&dir).await;
    (host.ctx.communities).set_governance_state("c1", closed());
    let mut events = host.ctx.journal.subscribe(None);
    let got = HostChannel::new(&host.ctx)
        .ingest(post("m2", "behind the overwrite"))
        .await
        .unwrap();
    assert_eq!(got, Ingested::Withheld);
    assert!(!stored(&host, &owner, "behind the overwrite").await);
    assert!(!published(&channel_events(&mut events).await, "m2"));
}

#[tokio::test]
async fn an_allowed_member_stores_and_receives_the_post() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = reader(&dir).await;
    (host.ctx.communities).set_governance_state("c1", with_channel());
    let mut events = host.ctx.journal.subscribe(None);
    let got = HostChannel::new(&host.ctx)
        .ingest(post("m2", "open to all"))
        .await
        .unwrap();
    assert!(matches!(got, Ingested::Stored { .. }), "{got:?}");
    assert!(stored(&host, &owner, "open to all").await);
    assert!(published(&channel_events(&mut events).await, "m2"));
}

#[tokio::test]
async fn a_later_view_grant_makes_the_withheld_post_readable() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = reader(&dir).await;
    (host.ctx.communities).set_governance_state("c1", closed());
    let channel = HostChannel::new(&host.ctx);
    let got = channel.ingest(post("m2", "granted later")).await.unwrap();
    assert_eq!(got, Ingested::Withheld);
    // The grant is a view gained, which re-reads every outbox; the re-read
    // hands the same entry to the ingest again.
    let (prev, next) = (closed(), with_channel());
    assert!(view_gained(Some(&prev), &next, "", 0));
    (host.ctx.communities).set_governance_state("c1", next);
    let mut events = host.ctx.journal.subscribe(None);
    let got = channel.ingest(post("m2", "granted later")).await.unwrap();
    assert!(matches!(got, Ingested::Stored { .. }), "{got:?}");
    assert!(stored(&host, &owner, "granted later").await);
    assert!(published(&channel_events(&mut events).await, "m2"));
}

#[test]
fn only_an_opened_channel_is_a_view_gained() {
    let (open, shut) = (with_channel(), closed());
    assert!(!view_gained(Some(&open), &open, "", 0));
    assert!(!view_gained(Some(&open), &shut, "", 0));
    assert!(!view_gained(None, &open, "", 0));
}

/// Give us the community key `me` on `c1`.
fn set_me(host: &Host, me: &PseudonymKey) {
    let mut map = host.ctx.communities.map().write();
    map.get_mut("c1").expect("c1").my_pseudonym_key = Some(hex::encode(me.0));
}

#[tokio::test]
async fn history_of_a_channel_closed_to_us_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _owner) = reader(&dir).await;
    set_me(&host, &PseudonymKey([0x42; 32]));
    (host.ctx.communities).set_governance_state("c1", closed());
    let got = HostChannel::new(&host.ctx)
        .history("c1", EDIT_CHANNEL, None, 10)
        .await;
    let Err(ChannelError::PermissionDenied(reason)) = got else {
        panic!("expected a permission refusal, got {got:?}");
    };
    assert_eq!(
        reason,
        "Read History is turned off for you in this channel."
    );
}

#[tokio::test]
async fn community_permission_check_names_every_missing_bit() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _owner) = reader(&dir).await;
    set_me(&host, &PseudonymKey([0x42; 32]));
    let required = VIEW_CHANNELS | MANAGE_CHANNELS | MANAGE_ROLES;
    let gov = host.governance();
    host.ctx
        .communities
        .map()
        .write()
        .get_mut("c1")
        .unwrap()
        .governance_state = None;
    assert_eq!(
        gov.require_permission("c1", required)
            .unwrap_err()
            .to_string(),
        "This community's rules have not loaded yet."
    );

    host.ctx.communities.set_governance_state("c1", closed());
    assert_eq!(
        gov.require_permission("c1", required)
            .unwrap_err()
            .to_string(),
        "You need the Manage Channels permission. You need the Manage Roles permission."
    );
}

#[tokio::test]
async fn history_of_a_closed_channel_is_open_to_the_member_it_admits() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _owner) = reader(&dir).await;
    let me = PseudonymKey([0x42; 32]);
    set_me(&host, &me);
    let mut gov = closed();
    gov.overwrites.insert(
        (
            ChannelId::from_hex(EDIT_CHANNEL).expect("channel id"),
            OverwriteTarget::Member(me).key(),
        ),
        OverwriteState {
            target_type: "member".into(),
            allow: VIEW_CHANNELS | READ_HISTORY,
            deny: 0,
            stamp: rekindle_governance::fixture::stamp(2),
        },
    );
    (host.ctx.communities).set_governance_state("c1", gov);
    let got = HostChannel::new(&host.ctx)
        .history("c1", EDIT_CHANNEL, None, 10)
        .await;
    assert!(
        !matches!(got, Err(ChannelError::PermissionDenied(_))),
        "{got:?}"
    );
}
