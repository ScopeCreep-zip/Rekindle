use rekindle_governance::op::OpStamp;
use rekindle_governance::state::{AutoModRuleState, ChannelState, TimeoutState};
use rekindle_types::governance::OpHash;

use rekindle_types::governance::AutoModAction;

use super::*;

/// The stamp of a test op at `lamport`.
fn stamp(lamport: u64) -> OpStamp {
    OpStamp {
        lamport,
        hash: OpHash([0; 32]),
    }
}

const GATE_CHANNEL: &str = "00112233445566778899aabbccddeeff";
const ANN: &str = "aa00000000000000000000000000000000000000000000000000000000000000";

fn item(body: &str, sent_at_secs: u64, now_secs: u64) -> GateItem<'_> {
    GateItem {
        community_id: "c",
        channel_id: GATE_CHANNEL,
        sender_pseudonym: ANN,
        body,
        sent_at_secs,
        held_before_secs: None,
        now_secs,
        backfill: false,
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
            created: OpStamp::MIN,
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
            stamp: stamp(1),
        },
    );
}

fn judge(gov: &GovernanceState, item: &GateItem<'_>) -> Verdict {
    gate(
        item,
        gov,
        &AutoModCompiledCache::compile(gov),
        &BlockList::default(),
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
            stamp: stamp(1),
        },
    );
    assert_eq!(
        judge(&gov, &item("hi", 120, 159)),
        Verdict::Drop(GateReason::TimedOut { until_secs: 160 })
    );
    assert_eq!(judge(&gov, &item("hi", 160, 160)), Verdict::Admit);
}

/// Judge `item` against the posts `held` holds, as the Host judges a
/// batch, and hold it unless it is dropped.
fn judge_held(gov: &GovernanceState, held: &mut BatchWindows, item: GateItem<'_>) -> Verdict {
    let item = GateItem {
        held_before_secs: held.held_before(item.sender_pseudonym, item.sent_at_secs, None),
        ..item
    };
    let verdict = gate(
        &item,
        gov,
        &AutoModCompiledCache::compile(gov),
        &BlockList::default(),
    );
    held.record(item.sender_pseudonym, item.sent_at_secs, &verdict);
    verdict
}

fn slowmode_state() -> GovernanceState {
    let mut gov = state();
    for channel in gov.channels.values_mut() {
        channel.slowmode_seconds = Some(30);
    }
    gov
}

#[test]
fn drops_a_post_within_slowmode_and_admits_after_it() {
    let gov = slowmode_state();
    let mut held = BatchWindows::default();
    assert_eq!(
        judge_held(&gov, &mut held, item("one", 100, 100)),
        Verdict::Admit
    );
    assert_eq!(
        judge_held(&gov, &mut held, item("two", 110, 110)),
        Verdict::Drop(GateReason::Slowmode { seconds: 30 })
    );
    assert_eq!(
        judge_held(&gov, &mut held, item("three", 130, 130)),
        Verdict::Admit
    );
}

#[test]
fn a_stored_post_is_the_window_when_it_is_the_nearest() {
    let gov = slowmode_state();
    let mut held = BatchWindows::default();
    let item = GateItem {
        held_before_secs: held.held_before(ANN, 110, Some(100)),
        ..item("two", 110, 110)
    };
    let gated = gate(
        &item,
        &gov,
        &AutoModCompiledCache::compile(&gov),
        &BlockList::default(),
    );
    assert_eq!(gated, Verdict::Drop(GateReason::Slowmode { seconds: 30 }));
    held.record(ANN, 110, &gated);
    assert_eq!(held.held_before(ANN, 200, Some(100)), Some(100));
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

fn thread_by(creator: &PseudonymKey) -> rekindle_governance::state::ThreadState {
    rekindle_governance::state::ThreadState {
        parent_channel_id: ChannelId([1; 16]),
        name: "side".into(),
        thread_type: "public".into(),
        record_key: None,
        invited: Vec::new(),
        forum_tag: None,
        auto_archive_seconds: 3600,
        creator: creator.clone(),
        created: stamp(2),
        archived: None,
        unarchived: None,
    }
}

fn time_out(gov: &mut GovernanceState, member: &PseudonymKey) {
    gov.timeouts.insert(
        member.clone(),
        TimeoutState {
            duration_seconds: 60,
            started_at: 100,
            stamp: stamp(1),
        },
    );
}

/// Another member's reply in a thread whose creator is timed out is
/// judged on its own author and admitted.
#[test]
fn a_reply_in_a_timed_out_creators_thread_is_admitted() {
    let mut gov = state();
    let creator = PseudonymKey([0xbb; 32]);
    gov.threads.insert(
        ThreadId(ChannelId::from_hex(GATE_CHANNEL).expect("thread id").0),
        thread_by(&creator),
    );
    time_out(&mut gov, &creator);
    assert_eq!(judge(&gov, &item("hi", 120, 120)), Verdict::Admit);
}

/// A thread received while its creator is timed out is held, and stays
/// held after the timeout ends; one already known, or by another member,
/// is kept.
#[test]
fn a_thread_received_while_its_creator_is_timed_out_is_held() {
    let (late, other) = (PseudonymKey([0xbb; 32]), PseudonymKey([0xcc; 32]));
    let mut known = state();
    known.threads.insert(ThreadId([1; 16]), thread_by(&late));
    let mut merged = known.clone();
    merged.threads.insert(ThreadId([2; 16]), thread_by(&late));
    merged.threads.insert(ThreadId([3; 16]), thread_by(&other));
    time_out(&mut merged, &late);
    let mut held = HashSet::new();
    hold_timed_out_threads(Some(&known), &mut held, &mut merged, 120);
    assert_eq!(held, HashSet::from([ThreadId([2; 16])]));
    let mut kept: Vec<_> = merged.threads.keys().map(|id| id.0[0]).collect();
    kept.sort_unstable();
    assert_eq!(kept, vec![1, 3]);
    merged.threads.insert(ThreadId([2; 16]), thread_by(&late));
    hold_timed_out_threads(Some(&known), &mut held, &mut merged, 500);
    assert!(!merged.threads.contains_key(&ThreadId([2; 16])));
}

/// Judge posts at `times` (seconds) in that order under a 30 s slowmode,
/// answering each post's verdict by its timestamp.
fn slowmode_verdicts(times: &[u64]) -> BTreeMap<u64, Verdict> {
    let gov = slowmode_state();
    let mut held = BatchWindows::default();
    (times.iter())
        .map(|t| (*t, judge_held(&gov, &mut held, item("post", *t, 500))))
        .collect()
}

#[test]
fn an_older_post_judged_after_a_newer_one_gets_its_in_order_verdict() {
    let in_order = slowmode_verdicts(&[100, 140]);
    assert_eq!(in_order[&100], Verdict::Admit);
    assert_eq!(in_order[&140], Verdict::Admit);
    assert_eq!(slowmode_verdicts(&[140, 100]), in_order);
}

#[test]
fn the_later_of_two_posts_inside_the_window_drops_whatever_the_arrival_order() {
    let slowmode = Verdict::Drop(GateReason::Slowmode { seconds: 30 });
    let in_order = slowmode_verdicts(&[100, 110, 200]);
    assert_eq!(in_order[&100], Verdict::Admit);
    assert_eq!(in_order[&110], slowmode);
    assert_eq!(in_order[&200], Verdict::Admit);
    for order in [[100, 200, 110], [200, 100, 110]] {
        assert_eq!(slowmode_verdicts(&order), in_order, "{order:?}");
    }
}

#[test]
fn a_predecessor_arriving_after_its_successor_does_not_drop_it() {
    let verdicts = slowmode_verdicts(&[110, 100]);
    assert_eq!(verdicts[&110], Verdict::Admit);
    assert_eq!(verdicts[&100], Verdict::Admit);
}

/// ANN joined (an op log is known), then departed at second 100.
fn departed_state() -> GovernanceState {
    let mut gov = state();
    let ann = PseudonymKey::from_hex_lossy(ANN);
    gov.op_log_records.insert(ann.clone(), "VLD0:m".into());
    gov.departed_at.insert(ann, 100);
    gov
}

#[test]
fn a_departed_authors_live_post_is_dropped() {
    let gov = departed_state();
    for sent in [50, 150] {
        assert_eq!(
            judge(&gov, &item("hi", sent, 200)),
            Verdict::Drop(GateReason::NotMember)
        );
    }
}

#[test]
fn a_backfilled_post_dated_before_the_departure_stays_readable() {
    let gov = departed_state();
    let before = GateItem {
        backfill: true,
        ..item("old", 50, 200)
    };
    let after = GateItem {
        backfill: true,
        ..item("new", 100, 200)
    };
    assert_eq!(judge(&gov, &before), Verdict::Admit);
    assert_eq!(judge(&gov, &after), Verdict::Drop(GateReason::NotMember));
}

#[test]
fn a_rejoined_member_is_admitted() {
    let mut gov = departed_state();
    gov.members.insert(
        PseudonymKey::from_hex_lossy(ANN),
        rekindle_governance::state::MemberState {
            member_record: "VLD0:m".into(),
            member_record_owner: "owner".into(),
            joined: rekindle_governance::op::OpStamp::MIN,
        },
    );
    assert_eq!(judge(&gov, &item("back", 150, 200)), Verdict::Admit);
}
