use rekindle_governance::state::{AutoModRuleState, ChannelState, TimeoutState};

use rekindle_types::governance::AutoModAction;

use super::*;

const GATE_CHANNEL: &str = "00112233445566778899aabbccddeeff";
const ANN: &str = "aa00000000000000000000000000000000000000000000000000000000000000";

fn item(body: &str, sent_at_secs: u64, now_secs: u64) -> GateItem<'_> {
    GateItem {
        community_id: "c",
        channel_id: GATE_CHANNEL,
        sender_pseudonym: ANN,
        body,
        sent_at_secs,
        now_secs,
    }
}

fn state() -> GovernanceState {
    let mut gov = GovernanceState::default();
    gov.channels.insert(
        ChannelId::from_hex(GATE_CHANNEL).expect("channel id"),
        ChannelState {
            name: "general".into(),
            channel_type: "text".into(),
            record_key: String::new(),
            category_id: None,
            position: 0,
            topic: None,
            forum_tags: None,
            slowmode_seconds: None,
            nsfw: None,
            parent_voice_channel_id: None,
            created_lamport: 0,
        },
    );
    gov
}

fn rule(gov: &mut GovernanceState, id: u8, trigger_json: &str, action: AutoModAction) {
    gov.automod_rules.insert(
        [id; 16],
        AutoModRuleState {
            name: format!("rule {id}"),
            enabled: true,
            trigger_json: trigger_json.into(),
            action,
            lamport: 1,
        },
    );
}

fn judge(gov: &GovernanceState, item: &GateItem<'_>) -> Verdict {
    gate(
        item,
        gov,
        &AutoModCompiledCache::compile(gov),
        &BlockList::default(),
        &mut RateLimits::default(),
    )
}

#[test]
fn admits_a_plain_post() {
    assert_eq!(judge(&state(), &item("hi", 10, 10)), Verdict::Admit);
}

#[test]
fn drops_a_blocked_author() {
    let blocked = BlockList::new([ANN.to_ascii_uppercase()]);
    let verdict = gate(
        &item("hi", 10, 10),
        &state(),
        &AutoModCompiledCache::compile(&state()),
        &blocked,
        &mut RateLimits::default(),
    );
    assert_eq!(verdict, Verdict::Drop(GateReason::Blocked));
}

#[test]
fn drops_a_banned_author() {
    let mut gov = state();
    gov.bans.insert(PseudonymKey::from_hex_lossy(ANN));
    assert_eq!(
        judge(&gov, &item("hi", 10, 10)),
        Verdict::Drop(GateReason::Banned)
    );
}

#[test]
fn drops_a_timed_out_author_until_the_timeout_ends() {
    let mut gov = state();
    gov.timeouts.insert(
        PseudonymKey::from_hex_lossy(ANN),
        TimeoutState {
            duration_seconds: 60,
            started_at: 100,
            lamport: 1,
        },
    );
    assert_eq!(
        judge(&gov, &item("hi", 120, 159)),
        Verdict::Drop(GateReason::TimedOut { until_secs: 160 })
    );
    assert_eq!(judge(&gov, &item("hi", 160, 160)), Verdict::Admit);
}

#[test]
fn drops_a_post_within_slowmode_and_admits_after_it() {
    let mut gov = state();
    for channel in gov.channels.values_mut() {
        channel.slowmode_seconds = Some(30);
    }
    let (blocked, mut limits) = (BlockList::default(), RateLimits::default());
    let rules = AutoModCompiledCache::compile(&gov);
    assert_eq!(
        gate(&item("one", 100, 100), &gov, &rules, &blocked, &mut limits),
        Verdict::Admit
    );
    assert_eq!(
        gate(&item("two", 110, 110), &gov, &rules, &blocked, &mut limits),
        Verdict::Drop(GateReason::Slowmode { seconds: 30 })
    );
    assert_eq!(
        gate(
            &item("three", 130, 130),
            &gov,
            &rules,
            &blocked,
            &mut limits
        ),
        Verdict::Admit
    );
}

#[test]
fn automod_blocks_blurs_and_alerts() {
    let mut gov = state();
    rule(
        &mut gov,
        1,
        r#"{"keywords":["spam"]}"#,
        AutoModAction::BlockLocally,
    );
    rule(
        &mut gov,
        2,
        r#"{"keywords":["rude"]}"#,
        AutoModAction::BlurContent,
    );
    rule(
        &mut gov,
        3,
        r#"{"keywords":["help"]}"#,
        AutoModAction::AlertModerators,
    );
    let named = |id: u8| GateReason::AutoMod {
        rule: format!("rule {id}"),
    };
    assert_eq!(
        judge(&gov, &item("SPAM and rude", 1, 1)),
        Verdict::Drop(named(1))
    );
    assert_eq!(judge(&gov, &item("so rude", 1, 1)), Verdict::Hide(named(2)));
    assert_eq!(
        judge(&gov, &item("help me", 1, 1)),
        Verdict::Report(named(3))
    );
}

#[test]
fn an_automod_rule_that_cannot_be_evaluated_drops() {
    let mut gov = state();
    rule(
        &mut gov,
        1,
        r#"{"regexPatterns":["("]}"#,
        AutoModAction::BlurContent,
    );
    assert!(matches!(
        judge(&gov, &item("anything", 1, 1)),
        Verdict::Drop(GateReason::AutoModError(_))
    ));
}

#[test]
fn drops_a_post_in_a_thread_whose_creator_is_timed_out() {
    let mut gov = state();
    let creator = PseudonymKey([0xbb; 32]);
    gov.threads.insert(
        ThreadId(ChannelId::from_hex(GATE_CHANNEL).expect("thread id").0),
        rekindle_governance::state::ThreadState {
            parent_channel_id: ChannelId([1; 16]),
            name: "side".into(),
            thread_type: "public".into(),
            record_key: None,
            invited: Vec::new(),
            forum_tag: None,
            auto_archive_seconds: 3600,
            creator: creator.clone(),
            created_lamport: 2,
            archived_lamport: None,
        },
    );
    gov.timeouts.insert(
        creator,
        TimeoutState {
            duration_seconds: 60,
            started_at: 100,
            lamport: 1,
        },
    );
    assert_eq!(
        judge(&gov, &item("hi", 120, 120)),
        Verdict::Drop(GateReason::TimedOut { until_secs: 160 })
    );
    assert_eq!(judge(&gov, &item("hi", 160, 160)), Verdict::Admit);
}
