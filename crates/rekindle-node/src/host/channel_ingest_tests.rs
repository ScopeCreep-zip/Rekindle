//! The Host ingest's gate (plan E3.1 F3, D26), offline: a banned or
//! timed-out author, a second post inside the slowmode, a blocked author
//! and an automod `block_locally` match are dropped; a rule that cannot be
//! compiled drops the post and is compiled once, not per post.

use std::sync::Arc;

use rekindle_channel::ingest::{GateReason, Verdict};
use rekindle_codec::community::channel_record::ChannelMessage;
use rekindle_governance::state::{AutoModRuleState, ChannelState, GovernanceState, TimeoutState};
use rekindle_types::governance::AutoModAction;
use rekindle_types::id::{ChannelId, PseudonymKey};

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::channel::ingest::ReceivedPost;
use crate::host::adapters::channel::HostChannel;
use crate::host::Host;

const INGEST_CHANNEL: &str = "0123456789abcdef0123456789abcdef";
const BEA: [u8; 32] = [0xbe; 32];

fn governance(slowmode_seconds: Option<u32>) -> GovernanceState {
    let mut gov = GovernanceState::default();
    gov.channels.insert(
        ChannelId::from_hex(INGEST_CHANNEL).expect("channel id"),
        ChannelState {
            name: "general".into(),
            channel_type: "text".into(),
            record_key: String::new(),
            category_id: None,
            position: 0,
            topic: None,
            forum_tags: None,
            slowmode_seconds,
            nsfw: None,
            parent_voice_channel_id: None,
            created_lamport: 0,
        },
    );
    gov
}

fn rule(gov: &mut GovernanceState, trigger_json: &str) {
    gov.automod_rules.insert(
        [7; 16],
        AutoModRuleState {
            name: "no spam".into(),
            enabled: true,
            trigger_json: trigger_json.into(),
            action: AutoModAction::BlockLocally,
            lamport: 1,
        },
    );
}

fn post(timestamp_secs: u64) -> ReceivedPost {
    ReceivedPost {
        community_id: "c1".into(),
        channel_id: INGEST_CHANNEL.into(),
        record_key: "VLD0:channel".into(),
        subkey_index: 1,
        sequence: 1,
        message: ChannelMessage {
            sequence: 1,
            sender_pseudonym: hex::encode(BEA),
            ciphertext: Vec::new(),
            mek_generation: 0,
            timestamp: timestamp_secs * 1000,
            reply_to: None,
            lamport_ts: 1,
            message_id: Some(format!("m{timestamp_secs}")),
            attachment: None,
            flags: 0,
            mentioned_pseudonyms: Vec::new(),
            mentioned_roles: Vec::new(),
        },
        forwarded_from_author: None,
        attachment_json: None,
    }
}

/// An unlocked Host holding `c1` under `gov`, and our owner key.
async fn host_under(dir: &tempfile::TempDir, gov: GovernanceState) -> (Arc<Host>, String) {
    let host = offline_host(dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    host.ctx.communities.set_governance_state("c1", gov);
    (host, owner)
}

async fn judge(host: &Host, owner: &str, post: &ReceivedPost, body: &str) -> Verdict {
    let db = host.ctx.db.current().unwrap();
    HostChannel::new(&host.ctx)
        .judge(owner, &db, post, body)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_banned_author_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let mut gov = governance(None);
    gov.bans.insert(PseudonymKey(BEA));
    let (host, owner) = host_under(&dir, gov).await;
    let now = host.ctx.clock.now_secs();
    let verdict = judge(&host, &owner, &post(now), "hi").await;
    assert_eq!(verdict, Verdict::Drop(GateReason::Banned));
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_timed_out_author_is_dropped_until_the_timeout_ends() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = host_under(&dir, GovernanceState::default()).await;
    let now = host.ctx.clock.now_secs();
    let mut gov = governance(None);
    gov.timeouts.insert(
        PseudonymKey(BEA),
        TimeoutState {
            duration_seconds: 600,
            started_at: now,
            lamport: 1,
        },
    );
    host.ctx.communities.set_governance_state("c1", gov.clone());
    let verdict = judge(&host, &owner, &post(now), "hi").await;
    assert_eq!(
        verdict,
        Verdict::Drop(GateReason::TimedOut {
            until_secs: now + 600
        })
    );
    gov.timeouts.get_mut(&PseudonymKey(BEA)).unwrap().started_at = now - 600;
    host.ctx.communities.set_governance_state("c1", gov);
    assert_eq!(judge(&host, &owner, &post(now), "hi").await, Verdict::Admit);
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_second_post_inside_the_slowmode_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = host_under(&dir, governance(Some(30))).await;
    let now = host.ctx.clock.now_secs();
    assert_eq!(
        judge(&host, &owner, &post(now), "one").await,
        Verdict::Admit
    );
    assert_eq!(
        judge(&host, &owner, &post(now + 5), "two").await,
        Verdict::Drop(GateReason::Slowmode { seconds: 30 })
    );
    assert_eq!(
        judge(&host, &owner, &post(now + 30), "three").await,
        Verdict::Admit
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_blocked_author_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = host_under(&dir, governance(None)).await;
    let o = owner.clone();
    (host.ctx.db.current().unwrap())
        .call(move |conn| {
            rekindle_db::repo::community_blocks::insert(conn, &o, "c1", &hex::encode(BEA), 1)
        })
        .await
        .unwrap();
    let now = host.ctx.clock.now_secs();
    let verdict = judge(&host, &owner, &post(now), "hi").await;
    assert_eq!(verdict, Verdict::Drop(GateReason::Blocked));
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_automod_block_locally_match_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let mut gov = governance(None);
    rule(&mut gov, r#"{"keywords":["spam"],"regexPatterns":[]}"#);
    let (host, owner) = host_under(&dir, gov).await;
    let now = host.ctx.clock.now_secs();
    assert_eq!(
        judge(&host, &owner, &post(now), "buy spam").await,
        Verdict::Drop(GateReason::AutoMod {
            rule: "no spam".into()
        })
    );
    assert_eq!(
        judge(&host, &owner, &post(now + 1), "hello").await,
        Verdict::Admit
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn an_uncompilable_rule_drops_and_is_compiled_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut gov = governance(None);
    rule(&mut gov, r#"{"keywords":[],"regexPatterns":["("]}"#);
    let (host, owner) = host_under(&dir, gov).await;
    let now = host.ctx.clock.now_secs();
    let first = judge(&host, &owner, &post(now), "hi").await;
    assert!(
        matches!(first, Verdict::Drop(GateReason::AutoModError(_))),
        "{first:?}"
    );
    let cached = host.ctx.automod_rules.read().get("c1").cloned().unwrap();
    let second = judge(&host, &owner, &post(now + 1), "again").await;
    assert!(matches!(second, Verdict::Drop(GateReason::AutoModError(_))));
    let still = host.ctx.automod_rules.read().get("c1").cloned().unwrap();
    assert!(Arc::ptr_eq(&cached, &still), "not recompiled per post");
    host.shutdown().await.unwrap();
}
