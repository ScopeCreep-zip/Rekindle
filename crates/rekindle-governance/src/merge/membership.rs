//! Membership ops (plan E3.2): `MemberJoined` (absorbs `JoinRequested`),
//! `MemberRecordMoved`, `MemberLeft` and the durable kick `MemberRemoved`.
//! Under `ApprovalRequired` a joiner waits in `pending_members` until a
//! `MemberApproved`; a `MemberRejected` drops it.

use rekindle_types::governance::{AdmissionMode, GovernanceEntry};
use rekindle_types::id::PseudonymKey;

use crate::op::OpStamp;
use crate::state::{GovernanceState, MemberState, PendingMemberState};

pub(super) fn apply_membership(
    author: &PseudonymKey,
    entry: &GovernanceEntry,
    stamp: OpStamp,
    state: &mut GovernanceState,
) {
    match entry {
        GovernanceEntry::MemberJoined {
            member_record,
            member_record_owner,
            ..
        } => {
            state
                .op_log_records
                .insert(author.clone(), member_record.clone());
            if state.removed.get(author).is_some_and(|r| *r > stamp) {
                return;
            }
            let member = MemberState {
                member_record: member_record.clone(),
                member_record_owner: member_record_owner.clone(),
                joined: stamp,
            };
            let needs_approval = state.effective_admission_mode()
                == AdmissionMode::ApprovalRequired
                && state.creator.as_ref() != Some(author);
            match state.admitted.get(author).map(|d| d.approved) {
                _ if !needs_approval => {
                    state.members.insert(author.clone(), member);
                }
                Some(true) => {
                    state.members.insert(author.clone(), member);
                }
                Some(false) => {}
                None => {
                    state.pending_members.insert(
                        author.clone(),
                        PendingMemberState {
                            member_record: member.member_record,
                            member_record_owner: member.member_record_owner,
                            stamp,
                        },
                    );
                }
            }
        }
        GovernanceEntry::MemberRecordMoved {
            member_record,
            member_record_owner,
        } => {
            if let Some(member) = state.members.get_mut(author) {
                state
                    .op_log_records
                    .insert(author.clone(), member_record.clone());
                member.member_record.clone_from(member_record);
                member.member_record_owner.clone_from(member_record_owner);
            }
        }
        GovernanceEntry::MemberLeft {} => {
            state.members.remove(author);
            state.pending_members.remove(author);
        }
        GovernanceEntry::MemberRemoved { target, .. } => {
            state.members.remove(target);
            state.pending_members.remove(target);
            let slot = state.removed.entry(target.clone()).or_insert(stamp);
            *slot = (*slot).max(stamp);
        }
        _ => {}
    }
}
