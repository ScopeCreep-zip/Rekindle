use rekindle_governance::state::TimeoutState;
use rekindle_types::id::{ChannelId, ThreadId};

use super::*;

fn thread() -> GovernanceEntry {
    GovernanceEntry::ThreadCreated {
        thread_id: ThreadId([4; 16]),
        parent_channel_id: ChannelId([5; 16]),
        name: "side".into(),
        thread_type: "public".into(),
        invited: Vec::new(),
        forum_tag: None,
        auto_archive_seconds: 3600,
    }
}

fn timed_out(member: PseudonymKey) -> GovernanceState {
    let mut gov = GovernanceState::default();
    gov.timeouts.insert(
        member,
        TimeoutState {
            duration_seconds: 60,
            started_at: 100,
            reason: None,
            stamp: rekindle_governance::fixture::stamp(1),
        },
    );
    gov
}

#[test]
fn a_timed_out_member_creates_no_thread_until_the_timeout_ends() {
    let member = PseudonymKey([2; 32]);
    let gov = timed_out(member.clone());
    let refused = refuse_timed_out_thread(&thread(), &gov, &member, 159).unwrap_err();
    assert!(
        matches!(&refused, ChannelError::PermissionDenied(m) if m.contains("160")),
        "{refused}"
    );
    assert!(refuse_timed_out_thread(&thread(), &gov, &member, 160).is_ok());
}

#[test]
fn other_members_and_other_entries_pass() {
    let member = PseudonymKey([2; 32]);
    let gov = timed_out(member.clone());
    assert!(refuse_timed_out_thread(&thread(), &gov, &PseudonymKey([3; 32]), 120).is_ok());
    let archive = GovernanceEntry::ThreadArchived {
        thread_id: ThreadId([4; 16]),
    };
    assert!(refuse_timed_out_thread(&archive, &gov, &member, 120).is_ok());
}
