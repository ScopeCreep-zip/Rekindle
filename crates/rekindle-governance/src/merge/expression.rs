//! `merge` expression CRDT apply rules.

use crate::op::OpStamp;

use super::{ExpressionState, GovernanceEntry, GovernanceState};

pub(super) fn apply_expression(entry: &GovernanceEntry, stamp: OpStamp, state: &mut GovernanceState) {
    match entry {
        GovernanceEntry::ExpressionAdded {
            expression_id,
            name,
            kind,
            content_hash,
            attachment,
            animated,
            tags,
            sound_meta,
            creator_pseudonym,
            created_at,
            available_to_peers,
        } => {
            let removed_stamp = state
                .expression_remove_stamps
                .get(expression_id)
                .copied()
                .unwrap_or(OpStamp::MIN);
            let existing_stamp = state
                .expressions
                .get(expression_id)
                .map_or(OpStamp::MIN, |expr| expr.stamp);
            if stamp > removed_stamp && stamp > existing_stamp {
                state.expressions.insert(
                    *expression_id,
                    ExpressionState {
                        name: name.clone(),
                        kind: kind.clone(),
                        content_hash: content_hash.clone(),
                        attachment: attachment.clone(),
                        animated: *animated,
                        tags: tags.clone(),
                        sound_meta: sound_meta.clone(),
                        creator_pseudonym: creator_pseudonym.clone(),
                        created_at: *created_at,
                        available_to_peers: available_to_peers.unwrap_or(true),
                        stamp,
                    },
                );
            }
        }

        GovernanceEntry::ExpressionRemoved {
            expression_id,
        } => {
            let removed_stamp = state
                .expression_remove_stamps
                .get(expression_id)
                .copied()
                .unwrap_or(OpStamp::MIN);
            if stamp > removed_stamp {
                state
                    .expression_remove_stamps
                    .insert(*expression_id, stamp);
            }
            if let Some(expression) = state.expressions.get(expression_id) {
                if stamp > expression.stamp {
                    state.expressions.remove(expression_id);
                }
            }
        }

        // ── Event archived (tombstone) ──
        GovernanceEntry::AttachmentPinned {
            attachment_id,
            pinned,
        } => {
            let entry_stamp = stamp;
            let prev = state
                .attachment_pin_stamps
                .get(attachment_id)
                .copied()
                .unwrap_or(OpStamp::MIN);
            if entry_stamp >= prev {
                state
                    .attachment_pin_stamps
                    .insert(*attachment_id, entry_stamp);
                if *pinned {
                    state.pinned_attachments.insert(*attachment_id);
                } else {
                    state.pinned_attachments.remove(attachment_id);
                }
            }
        }

        // ── Segment expansion: tracked for join flow discovery ──
        _ => unreachable!("apply_expression: unexpected variant"),
    }
}
