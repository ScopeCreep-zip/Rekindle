//! A friend rotated their identity (plan E2.3, `IdentityRotated`): their
//! friend row, our direct conversation with them and their
//! `signal_identities` row move from the old key to the new one in one
//! transaction, nothing queued to the old key is kept (it was sealed to a
//! session the new identity does not hold), the old session goes (a
//! record is bound to the identity its AD names), and so does their
//! mailbox key: that record is owned by the old identity, which writes it
//! no more (their route stays on the profile, whose owner carries over).
//! What else is keyed by their identity moves too (audit 2026-10-20 R-3):
//! missed calls and the strand relay we volunteered for them.
//! `peer_reliability` is keyed by a community pseudonym, not the identity,
//! so it stays. Their chain to us (plan E2.5) is on a record whose owner
//! is theirs and carries over: our cursor on it moves with them (the
//! entries already passed, framed by the old identity, are not read
//! again), as do the client ids applied from it and our stream's last seq
//! (a seq is never handed out twice). Our own record to them is replaced
//! at our next frame (its writer is saved under the old key), so its chain
//! bookkeeping goes. The rows keyed by their friend row (game totals,
//! the activity feed's entries and last-seen state, the DM read marker)
//! move with it.

use rusqlite::{params, Connection};

use super::{identity_text, signal_identities};

/// Move the friend `old` to `new` (both signatures checked by the
/// caller); whether `old` was a friend (nothing moves otherwise).
///
/// # Errors
/// A write failed; nothing moved.
pub fn rotate_peer(
    conn: &Connection,
    owner: &[u8; 32],
    old: &[u8; 32],
    new: &[u8; 32],
    now_ms: i64,
) -> rusqlite::Result<bool> {
    let (owner_key, old_key, new_key) =
        (identity_text(owner), identity_text(old), identity_text(new));
    let tx = conn.unchecked_transaction()?;
    // The rows keyed by the friend row (their game totals, the activity
    // feed's entries and last-seen state, the DM read marker) move with it
    // below; their foreign keys are checked at commit.
    tx.execute_batch("PRAGMA defer_foreign_keys = ON;")?;
    let moved = tx.execute(
        "UPDATE friends SET public_key = ?3, mailbox_dht_key = NULL \
         WHERE owner_key = ?1 AND public_key = ?2",
        params![owner_key, old_key, new_key],
    )? > 0;
    if moved {
        tx.execute(
            "UPDATE messages SET conversation_id = ?3 \
             WHERE owner_key = ?1 AND conversation_type = 'dm' AND conversation_id = ?2",
            params![owner_key, old_key, new_key],
        )?;
        tx.execute(
            "UPDATE messages SET sender_key = ?3 \
             WHERE owner_key = ?1 AND conversation_type = 'dm' AND sender_key = ?2",
            params![owner_key, old_key, new_key],
        )?;
        super::outbox::delete_to_peer(&tx, &owner_key, &old_key)?;
        for sql in [
            "UPDATE stream_cursors SET writer = ?3 \
             WHERE owner_key = ?1 AND stream = 'dm' AND writer = ?2",
            "UPDATE stream_applied SET writer = ?3 \
             WHERE owner_key = ?1 AND stream = 'dm' AND writer = ?2",
            "UPDATE received SET stream = 'dm:' || ?3 \
             WHERE owner_key = ?1 AND stream = 'dm:' || ?2",
            "UPDATE outbox_streams SET stream = 'dm:' || ?3 \
             WHERE owner_key = ?1 AND stream = 'dm:' || ?2",
        ] {
            tx.execute(sql, params![owner_key, old_key, new_key])?;
        }
        tx.execute(
            "DELETE FROM chain_entries WHERE owner_key = ?1 AND peer = ?2",
            params![owner_key, old_key],
        )?;
        crate::erase::delete(
            &tx,
            "DELETE FROM ratchet_sessions WHERE owner_key = ?1 AND peer_identity = ?2",
            params![owner_key, old_key],
        )?;
        for sql in [
            "UPDATE missed_calls SET peer_key = ?3 WHERE owner_key = ?1 AND peer_key = ?2",
            "UPDATE strand_relay_volunteered SET friend_public_key = ?3 \
             WHERE owner_key = ?1 AND friend_public_key = ?2",
        ] {
            tx.execute(sql, params![owner_key, old_key, new_key])?;
        }
        // What is keyed by their friend row is theirs and moves with it:
        // the game totals they shared (plan E7.2), what the activity feed
        // logged and last saw of them (plan E7.15) and our read marker on
        // their conversation (plan E3.1).
        for sql in [
            "UPDATE friend_game_totals SET friend_key = ?3 \
             WHERE owner_key = ?1 AND friend_key = ?2",
            "UPDATE activity_log SET subject_key = ?3 WHERE owner_key = ?1 AND subject_key = ?2",
            "UPDATE activity_seen SET friend_key = ?3 WHERE owner_key = ?1 AND friend_key = ?2",
            "UPDATE read_markers SET peer_key = ?3 WHERE owner_key = ?1 AND peer_key = ?2",
        ] {
            tx.execute(sql, params![owner_key, old_key, new_key])?;
        }
        super::friends::rekey_nickname(&tx, &owner_key, &old_key, &new_key)?;
        signal_identities::rotate(&tx, owner, old, new, now_ms)?;
    }
    tx.commit()?;
    Ok(moved)
}

#[cfg(test)]
#[path = "peer_rekey_tests.rs"]
mod tests;
