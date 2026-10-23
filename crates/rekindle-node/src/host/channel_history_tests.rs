//! A channel's history backfill (plan E3.1 E3f), offline: a record
//! message stored from the channel record keeps its attachment's record;
//! every backfilled post passes the D26 gate (a banned author's post is
//! neither stored nor shown, an automod `BlurContent` match is stored and
//! shown blurred, and a held row keeps its verdict).

use rekindle_channel::history::RecordMessage;
use rekindle_db::repo::messages::channel as rows;
use rekindle_governance::state::{AutoModRuleState, GovernanceState};
use rekindle_types::attachment::AttachmentOffer;
use rekindle_types::dto::channel_message::ChannelMessageView;
use rekindle_types::governance::AutoModAction;
use rekindle_types::id::PseudonymKey;
use rekindle_types::message::SendState;

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::channel::history::stored_view;
use crate::host::adapters::channel::HostChannel;
use crate::host::Host;

const BACKFILL_CHANNEL: &str = "0123456789abcdef0123456789abcdef";

fn offer() -> AttachmentOffer {
    AttachmentOffer {
        attachment_id: [0x5a; 16],
        filename: "map.png".into(),
        mime_type: "image/png".into(),
        total_size: 3,
        chunk_count: 1,
        chunk_size: 28 * 1024,
        merkle_root: [1; 32],
        chunk_hashes: vec![[2; 32]],
        wrapped_fek: vec![3; 60],
        fek_mek_generation: 0,
    }
}

fn record_message() -> RecordMessage {
    RecordMessage {
        view: ChannelMessageView {
            id: 0,
            sender_id: hex::encode([0xbe; 32]),
            body: "the map".into(),
            decryption_failed: false,
            automod_blurred: false,
            timestamp: 1_000,
            is_own: false,
            server_message_id: Some("m1".into()),
            reactions: None,
            pinned: None,
            poll: None,
            forwarded_from_author: None,
            attachment: None,
            flags: 0,
            send_state: SendState::Delivered,
        },
        reply_to: None,
        mek_generation: 0,
        lamport_ts: 1,
        mentions_me: false,
        attachment: Some(offer()),
    }
}

#[tokio::test]
async fn a_backfilled_message_keeps_its_attachment() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    host.ctx
        .communities
        .set_governance_state("c1", GovernanceState::default());
    backfill(&host, &owner).await;
    let stored = stored_rows(&host, &owner).await;
    assert_eq!(stored.len(), 1);
    let view = stored_view(stored.into_iter().next().unwrap(), &owner, "");
    let attachment = view.attachment.expect("the row keeps the attachment");
    assert_eq!(attachment.attachment_id, hex::encode([0x5a; 16]));
    assert_eq!(attachment.filename, "map.png");
    assert_eq!(attachment.chunk_count, 1);
    assert!(!attachment.downloaded);
    host.shutdown().await.unwrap();
}

async fn backfill(host: &Host, owner: &str) -> Vec<RecordMessage> {
    HostChannel::new(&host.ctx)
        .store_record_messages(owner, "c1", BACKFILL_CHANNEL, vec![record_message()])
        .await
        .unwrap()
}

async fn stored_rows(host: &Host, owner: &str) -> Vec<rows::StoredChannelMessage> {
    let db = host.ctx.db.current().unwrap();
    let key = owner.to_owned();
    db.call(move |conn| rows::page(conn, &key, BACKFILL_CHANNEL, None, 10))
        .await
        .unwrap()
}

/// The backfill judges each post through the same Host gate as the live
/// ingest: a banned author's post is neither stored nor shown.
#[tokio::test]
async fn a_banned_authors_backfilled_post_is_neither_stored_nor_shown() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let mut gov = GovernanceState::default();
    gov.bans.insert(PseudonymKey([0xbe; 32]));
    host.ctx.communities.set_governance_state("c1", gov);
    assert!(backfill(&host, &owner).await.is_empty(), "not shown");
    assert!(stored_rows(&host, &owner).await.is_empty(), "not stored");
    host.shutdown().await.unwrap();
}

/// A `BlurContent` match fetched by the backfill is stored with
/// `automod_blurred` and shown blurred; once held, the row's verdict is
/// what the next page shows.
#[tokio::test]
async fn a_hidden_backfilled_post_is_stored_and_shown_blurred() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let mut gov = GovernanceState::default();
    gov.automod_rules.insert(
        [7; 16],
        AutoModRuleState {
            name: "no maps".into(),
            enabled: true,
            trigger_json: r#"{"keywords":["map"],"regexPatterns":[]}"#.into(),
            action: AutoModAction::BlurContent,
            stamp: rekindle_governance::fixture::stamp(1),
        },
    );
    host.ctx.communities.set_governance_state("c1", gov);
    let shown = backfill(&host, &owner).await;
    assert_eq!(shown.len(), 1);
    assert!(shown[0].view.automod_blurred, "shown blurred");
    let stored = stored_rows(&host, &owner).await;
    assert!(stored[0].automod_blurred, "stored blurred");
    host.ctx
        .communities
        .set_governance_state("c1", GovernanceState::default());
    let again = backfill(&host, &owner).await;
    assert!(
        again[0].view.automod_blurred,
        "a held row keeps its verdict"
    );
    host.shutdown().await.unwrap();
}
