//! `merge` expression CRDT apply rules.

use super::{ExpressionState, GovernanceEntry, GovernanceState};

pub(super) fn apply_expression(entry: &GovernanceEntry, state: &mut GovernanceState) {
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
            lamport,
        } => {
            let removed_lamport = state
                .expression_remove_lamports
                .get(expression_id)
                .copied()
                .unwrap_or(0);
            let existing_lamport = state
                .expressions
                .get(expression_id)
                .map_or(0, |expr| expr.lamport);
            if *lamport > removed_lamport && *lamport > existing_lamport {
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
                        lamport: *lamport,
                    },
                );
            }
        }

        GovernanceEntry::ExpressionRemoved {
            expression_id,
            lamport,
        } => {
            let removed_lamport = state
                .expression_remove_lamports
                .get(expression_id)
                .copied()
                .unwrap_or(0);
            if *lamport > removed_lamport {
                state
                    .expression_remove_lamports
                    .insert(*expression_id, *lamport);
            }
            if let Some(expression) = state.expressions.get(expression_id) {
                if *lamport > expression.lamport {
                    state.expressions.remove(expression_id);
                }
            }
        }

        // ── Event archived (tombstone) ──
        GovernanceEntry::AttachmentPinned {
            attachment_id,
            pinned,
            lamport,
        } => {
            let entry_lamport = *lamport;
            let prev = state
                .attachment_pin_lamports
                .get(attachment_id)
                .copied()
                .unwrap_or(0);
            if entry_lamport >= prev {
                state
                    .attachment_pin_lamports
                    .insert(*attachment_id, entry_lamport);
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
