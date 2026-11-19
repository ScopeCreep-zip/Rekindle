//! An author's edits and deletes of its own channel messages (plan E3.3):
//! entries in the author's record, applied as events by the author and by
//! every reader under the rich-messaging rules.
//!
//! - **E1** an edit is valid only from the message's author, and the latest
//!   by the author's seq wins (no trusted clock).
//! - **E2** a reader caps a message at [`MAX_EDITS`] edits.
//! - **E3** an author tombstone is allowed at any time and is final: it
//!   purges every revision, and later edits are refused.
//! - **E4** the tombstone is "deleted for members", never "erased": the old
//!   ciphertext stays in the author's record and in peers' copies.
//!
//! An edit's body is sealed at the edit entry's own position (the record,
//! the author's slot and the edit's seq), never the message's.

use crate::deps::ChannelMessagingDeps;
use crate::send::SlotBodyPosition;

/// A reader caps a message at this many edits (E2).
pub const MAX_EDITS: u32 = 10;

/// The stored message a change names, as the rules judge it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeTarget {
    /// The message's author.
    pub sender_key: String,
    /// The author seq of the edit it shows; 0 when unedited.
    pub edit_seq: u64,
    /// How many edits were applied to it.
    pub edit_count: u32,
    /// Its author deleted it.
    pub deleted: bool,
}

/// What a change does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// An edit at the author's seq.
    Edit { seq: u64 },
    /// A tombstone.
    Delete,
}

/// Why a change was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ChangeRefusal {
    /// We hold no such message (an early change waits for its target, E5).
    #[error("no such message")]
    NoTarget,
    /// Only the message's author changes it (E1).
    #[error("only its author changes a message")]
    NotAuthor,
    /// Its author deleted it (E3).
    #[error("the message was deleted")]
    Deleted,
    /// The shown edit is at or above this seq (E1).
    #[error("a later edit is shown")]
    Stale,
    /// The message reached the edit cap (E2).
    #[error("the message reached {MAX_EDITS} edits")]
    EditLimit,
}

/// Judge a change by `author` of `target` under E1–E3.
///
/// # Errors
/// The rule that refuses it.
pub fn judge_change(
    target: Option<&ChangeTarget>,
    author: &str,
    kind: ChangeKind,
) -> Result<(), ChangeRefusal> {
    let target = judge_target(target, author)?;
    match kind {
        ChangeKind::Edit { seq } if seq <= target.edit_seq => Err(ChangeRefusal::Stale),
        ChangeKind::Edit { .. } if target.edit_count >= MAX_EDITS => Err(ChangeRefusal::EditLimit),
        ChangeKind::Edit { .. } | ChangeKind::Delete => Ok(()),
    }
}

/// Judge our own change of `target` (an edit when `edit`) under E1–E3
/// before it is queued: never stale, since its place is the next seq of
/// our channel's stream (D22), above every entry we wrote there; a reader
/// judges the seq ([`judge_change`]).
///
/// # Errors
/// The rule that refuses it.
pub fn judge_own_change(
    target: Option<&ChangeTarget>,
    author: &str,
    edit: bool,
) -> Result<(), ChangeRefusal> {
    let target = judge_target(target, author)?;
    if edit && target.edit_count >= MAX_EDITS {
        return Err(ChangeRefusal::EditLimit);
    }
    Ok(())
}

/// `target`, when it exists, is `author`'s and not deleted.
fn judge_target<'a>(
    target: Option<&'a ChangeTarget>,
    author: &str,
) -> Result<&'a ChangeTarget, ChangeRefusal> {
    let target = target.ok_or(ChangeRefusal::NoTarget)?;
    if !target.sender_key.eq_ignore_ascii_case(author) {
        return Err(ChangeRefusal::NotAuthor);
    }
    if target.deleted {
        return Err(ChangeRefusal::Deleted);
    }
    Ok(target)
}

/// Open an edit's body sealed at `at` under exactly `generation`; `None`
/// when we hold no key that opens it.
pub fn open_edit<D: ChannelMessagingDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    channel_id: &str,
    generation: u64,
    at: SlotBodyPosition<'_>,
    ciphertext: &[u8],
) -> Option<String> {
    crate::text_keys::open_text(deps, community_id, channel_id, generation, at, ciphertext)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(edit_seq: u64, edit_count: u32, deleted: bool) -> ChangeTarget {
        ChangeTarget {
            sender_key: "AAbb".into(),
            edit_seq,
            edit_count,
            deleted,
        }
    }

    #[test]
    fn only_the_author_changes_its_message() {
        let t = target(0, 0, false);
        assert_eq!(
            judge_change(Some(&t), "ccdd", ChangeKind::Edit { seq: 4 }),
            Err(ChangeRefusal::NotAuthor)
        );
        assert_eq!(
            judge_change(Some(&t), "ccdd", ChangeKind::Delete),
            Err(ChangeRefusal::NotAuthor)
        );
        assert_eq!(
            judge_change(Some(&t), "aabb", ChangeKind::Edit { seq: 4 }),
            Ok(())
        );
        assert_eq!(
            judge_change(None, "aabb", ChangeKind::Delete),
            Err(ChangeRefusal::NoTarget)
        );
    }

    #[test]
    fn the_latest_edit_by_seq_wins_up_to_the_cap() {
        assert_eq!(
            judge_change(
                Some(&target(7, 1, false)),
                "aabb",
                ChangeKind::Edit { seq: 7 }
            ),
            Err(ChangeRefusal::Stale)
        );
        assert_eq!(
            judge_change(
                Some(&target(7, MAX_EDITS, false)),
                "aabb",
                ChangeKind::Edit { seq: 9 }
            ),
            Err(ChangeRefusal::EditLimit)
        );
    }

    #[test]
    fn our_own_change_is_judged_without_a_seq() {
        // Our edit's place is our stream's next seq: no seq refuses it.
        assert_eq!(
            judge_own_change(Some(&target(u64::MAX, 1, false)), "aabb", true),
            Ok(())
        );
        assert_eq!(
            judge_own_change(Some(&target(7, MAX_EDITS, false)), "aabb", true),
            Err(ChangeRefusal::EditLimit)
        );
        assert_eq!(
            judge_own_change(Some(&target(7, MAX_EDITS, false)), "aabb", false),
            Ok(())
        );
        assert_eq!(
            judge_own_change(Some(&target(0, 0, true)), "aabb", false),
            Err(ChangeRefusal::Deleted)
        );
        assert_eq!(
            judge_own_change(Some(&target(0, 0, false)), "ccdd", true),
            Err(ChangeRefusal::NotAuthor)
        );
    }

    #[test]
    fn a_tombstone_is_final() {
        let t = target(7, MAX_EDITS, true);
        assert_eq!(
            judge_change(Some(&t), "aabb", ChangeKind::Edit { seq: 99 }),
            Err(ChangeRefusal::Deleted)
        );
        assert_eq!(
            judge_change(Some(&t), "aabb", ChangeKind::Delete),
            Err(ChangeRefusal::Deleted)
        );
        // A delete is allowed at any time before it, the cap reached or not.
        assert_eq!(
            judge_change(
                Some(&target(7, MAX_EDITS, false)),
                "aabb",
                ChangeKind::Delete
            ),
            Ok(())
        );
    }
}
