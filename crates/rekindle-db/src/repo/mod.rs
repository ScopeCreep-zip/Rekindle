//! Repositories: every query on the tables more than one host reads or
//! writes (plan C5.3, `steps-10-19.md` N1).
//!
//! Each function takes the connection and runs on the database thread,
//! inside a [`crate::Db::call`] closure, so several can share one
//! transaction (`rusqlite::Transaction` derefs to the connection). This is
//! the shape matrix-sdk-sqlite gives its table helpers
//! (`SqliteKeyValueStoreConnExt`, `SqliteTransactionExt` over
//! `rusqlite::Connection`). The SQL lives here and nowhere else.

pub mod activity;
pub mod audit;
pub mod bio;
pub mod blocked;
pub mod calls;
pub mod chain_entries;
pub mod channel_pins;
pub mod channels;
pub mod communities;
pub mod community_blocks;
pub mod community_events;
pub mod community_invites;
pub mod creator_round;
pub mod disappearing;
pub mod favourite_servers;
pub mod friend_game_totals;
pub mod friend_groups;
pub mod friends;
pub mod game_servers;
pub mod game_sessions;
pub mod governance_checkpoints;
pub mod governance_devices;
pub mod governance_ops;
pub mod governance_partial;
pub mod group_dm_invites;
pub mod group_dm_messages;
pub mod group_dm_rendezvous;
pub mod group_dms;
pub mod group_lobby;
pub mod held_threads;
pub mod hidden_games;
pub mod history_gap;
pub mod identity;
pub mod intent_facts;
pub mod members;
pub mod messages;
pub mod outbox;
pub mod outgoing_invites;
pub mod peer_rekey;
pub mod pending_requests;
pub mod preferences;
pub mod presence_keys;
pub mod read_markers;
pub mod received_dm;
pub mod recent_servers;
pub mod retry_requests;
pub mod route_budget;
pub mod search;
pub mod send_log;
pub mod session_reset_requests;
pub mod signal_identities;
pub mod skins;
pub mod stream_cursors;
pub mod threads;
pub mod transfers;
pub mod unprocessed;

#[cfg(test)]
mod constraints_tests;

/// An identity key as the per-owner tables store it: lowercase hex, the
/// form `identity.public_key` and `friends.public_key` hold.
pub(crate) fn identity_text(key: &[u8; 32]) -> String {
    hex::encode(key)
}

/// [`identity_text`] read back; anything else is a corrupt row naming
/// column `index`.
pub(crate) fn identity_from_text(text: &str, index: usize) -> rusqlite::Result<[u8; 32]> {
    let mut key = [0u8; 32];
    hex::decode_to_slice(text, &mut key).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            format!("identity key {text:?}: {e}").into(),
        )
    })?;
    Ok(key)
}

#[cfg(test)]
#[path = "mod_fixture.rs"]
pub(crate) mod fixture;

pub mod voice_session_events;
