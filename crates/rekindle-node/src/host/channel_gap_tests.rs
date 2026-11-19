//! The channel outbox's history-gap marker (plan E3.3, D23): a departure
//! head no read reached is a stored gap row in every channel we read,
//! stored once, shown by the history, and removed once a read reaches it;
//! each change is published so an open window follows it without a reload.

use rekindle_db::repo::messages::channel as rows;
use rekindle_governance::state::{ChannelState, GovernanceState};
use rekindle_types::id::ChannelId;
use rekindle_types::subscription_events::ChannelMessageEvent;

use super::channel_change_tests::{channel_events, reader, AUTHOR, EDIT_CHANNEL};
use crate::host::adapters::channel::HostChannel;
use crate::host::Host;

/// The merged state of `c1` with its one text channel.
fn with_channel() -> GovernanceState {
    let mut gov = GovernanceState::default();
    gov.channels.insert(
        ChannelId::from_hex(EDIT_CHANNEL).expect("channel id"),
        ChannelState {
            name: "general".into(),
            channel_type: "text".into(),
            category_id: None,
            position: 0,
            topic: None,
            forum_tags: None,
            slowmode_seconds: None,
            nsfw: None,
            parent_voice_channel_id: None,
            created: rekindle_governance::fixture::stamp(0),
        },
    );
    gov
}

/// The gap rows of the channel's history page: (sender, through).
async fn gaps(host: &Host, owner: &str) -> Vec<(String, u64)> {
    let owner = owner.to_owned();
    let page = (host.ctx.db.current().unwrap())
        .call(move |conn| rows::page(conn, &owner, EDIT_CHANNEL, None, 50))
        .await
        .unwrap();
    (page.into_iter())
        .filter_map(|row| Some((row.sender_key, row.history_gap_through?)))
        .collect()
}

#[tokio::test]
async fn an_unreached_departure_head_is_a_gap_row_until_a_read_reaches_it() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = reader(&dir).await;
    (host.ctx.communities).set_governance_state("c1", with_channel());
    let channel = HostChannel::new(&host.ctx);
    let mut events = host.ctx.journal.subscribe(None);
    channel.mark_outbox_gap("c1", &AUTHOR, 4).await;
    let author = hex::encode(AUTHOR.0);
    let got = channel_events(&mut events).await;
    assert!(
        matches!(&got[..], [ChannelMessageEvent::ChannelHistoryGap { community, channel, sender_pseudonym, through_seq: 4 }]
            if community == "c1" && channel == EDIT_CHANNEL && *sender_pseudonym == author),
        "an open window hears the gap it would read on a reload: {got:?}"
    );
    channel.mark_outbox_gap("c1", &AUTHOR, 4).await;
    assert_eq!(gaps(&host, &owner).await, vec![(author.clone(), 4)]);
    let view = (host.ctx.db.current().unwrap())
        .call({
            let owner = owner.clone();
            move |conn| rows::page(conn, &owner, EDIT_CHANNEL, None, 50)
        })
        .await
        .unwrap()
        .into_iter()
        .map(|row| crate::host::adapters::channel::history::stored_view(row, &owner, ""))
        .find(|view| view.history_gap_through.is_some())
        .expect("the history shows the gap");
    assert_eq!(view.sender_id, author);
    let _ = channel_events(&mut events).await;
    channel.close_outbox_gap("c1", &AUTHOR, 3).await;
    assert_eq!(
        gaps(&host, &owner).await.len(),
        1,
        "a read below the head closes nothing"
    );
    let got = channel_events(&mut events).await;
    assert!(got.is_empty(), "nothing closed, nothing heard: {got:?}");
    channel.close_outbox_gap("c1", &AUTHOR, 4).await;
    assert!(
        gaps(&host, &owner).await.is_empty(),
        "a read that reached the head closes it"
    );
    let got = channel_events(&mut events).await;
    assert!(
        matches!(&got[..], [ChannelMessageEvent::ChannelHistoryGapClosed { community, sender_pseudonym, through_seq: 4 }]
            if community == "c1" && *sender_pseudonym == author),
        "an open window drops the marker: {got:?}"
    );
}
