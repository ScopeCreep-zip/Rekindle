//! Opening an identity's database: the key, pragmas and the schema-version
//! check (plan D1).

use std::path::Path;

use rusqlite::Connection;

use crate::Db;

/// The schema. Edited in place before release; bump [`SCHEMA_VERSION`]
/// with every change (CLAUDE.md "No migration files").
pub const SCHEMA: &str = include_str!("../schema/001_init.sql");

/// Bump this every time `schema/001_init.sql` changes, or when a DHT record
/// layout is renumbered. It is also the data root's layout version
/// (`crate::reset`): a root stamped with another value is reset at startup,
/// Veilid storage included, so stale records written under the old layout
/// cannot be read back with the new indices. The app is not live yet.
///
/// 110: `group_dm_invites`, the verified group invites we have not
/// answered, held for `DELIVERY_HORIZON`, and `group_dm_chains`, each
/// member's announced group chain (plan E2.6 G2b).
///
/// 108: group DMs (plan E2.6 G1): the `group_dms` and
/// `group_dm_sender_keys` tables, `'group_dm'` in
/// `messages.conversation_type` and in `outbox.kind`.
///
/// 83: the durable record writes a logout left unsent (plan C7.6h); the
/// next login of the same identity re-pushes them (outbox rows since 103).
///
/// 81: the friend-list DHT record. `FriendEntry` gained `dm_log_key`
/// and `profileDhtKey`/`dmLogKey` joined `schemas/friend.capnp`, which
/// had only four fields while the Rust struct had five — the encoder
/// dropped `profile_dht_key` on every write and the decoder set it to
/// `None` with a comment saying so. The daemon track also wrote this
/// record in postcard while the desktop wrote Cap'n Proto, so a list
/// written by either is unreadable to the other.
///
/// 80: the account record's header shed the three child
/// `DHTShortArray` pointers and their owner keypairs. Creating an
/// account allocated a contact list, a chat list and an invitation list
/// that nothing ever wrote an entry to, and every login reopened all
/// three. `AccountHeader` is now the five fields it actually carries,
/// so a header written under the old schema decodes its text fields at
/// the wrong ordinals.
///
/// 79: the session membership record lost `is_operator`,
/// `community_mailbox_key` and `join_inbox_key`. All three were the
/// coordinator's, and the mailbox they named was never created by
/// anything — `create_community_mailbox` had no callers, so the route
/// the authority loop published into it went nowhere.
///
/// 76: `MemberPresence` gained `departed`, the signed tombstone a
/// leaver writes into its own registry slot so the slot can be reused.
/// Live rows are unaffected — the field is `skip_serializing_if`, so a
/// non-departed row stays byte-identical and old readers still verify
/// it. Only a departed row carries it, and an old reader drops the
/// unknown field before recomputing `signing_bytes()`, fails the
/// signature, and treats the slot as reclaimable too: both ends reach
/// the same answer. Bumped because the wire format grew a field, not
/// because the tracks disagree.
///
/// 75: the registry MEK vault is gone from community creation, and the
/// creator no longer writes itself into a shared member index. Both were
/// owner-subkey writes that `o_cnt: 0` grants nobody a credential for,
/// and `communities-channels.md` says the MEK is *never* written to DHT.
/// A community created under the old flow therefore carries a vault and
/// an index entry that new peers neither write nor read, while its
/// genesis MEK sits on the DHT where it does not belong. Rotation is now
/// peer-to-peer for both shells (`rekindle-mek-rotation`), delivered by
/// `app_call` rather than published.
///
/// 74: admission governance entries. `GovernanceEntry` gained
/// `JoinRequested`, `MemberApproved`, `MemberRejected` and
/// `AdmissionPolicy` (Cap'n Proto union arms 34-37), so a peer on the
/// old schema cannot read entries written by a new one — the union
/// discriminant is unknown to it. Also flips the daemon to the v2.0
/// self-sovereign join: members claim their own registry slot from the
/// invite's shared slot seed instead of being assigned one, so slots
/// written under the old flow used a locally-derived seed that no longer
/// resolves.
///
/// 73: profile DHT record renumbered. The two tracks had disagreed about
/// subkey 8 (Strand Relay pool vs friend-inbox key); the friend-inbox
/// pair moved to 9/10 and the record now allocates 11 subkeys. See
/// `rekindle_types::dht_layout::profile`.
/// 77: the member registry lost its last community-wide subkeys. The
/// member index (0), MEK vault (1) and moderation queue (5) were v1.0
/// owner subkeys that `o_cnt: 0` credentials nobody to write, and each
/// collided with a real member slot. Membership is now derived by
/// scanning signed presence rows, the MEK is delivered peer-to-peer and
/// never written to the DHT, and admission is a `GovernanceEntry`.
///
/// Landing with it: the daemon's channel storage moved from a
/// per-member DFLT `DhtLog` to the SMPL `(channel, segment)` records the
/// desktop already wrote, so the two tracks finally read and write the
/// same messages. Discovery is `ChannelCreated.record_key` plus
/// `ChannelSegmentLinked` out of merged governance, which is what made
/// the member index deletable.
/// 78: genesis entries gained `AdmissionPolicy` at lamport 1, shifting
/// the other five up by one. Every community created before this has an
/// entry set that cannot express `ApprovalRequired` — the variant was
/// merged, validated and Cap'n Proto encoded, and nothing ever wrote it,
/// so the mode was unreachable and every community was `Open`.
/// 84: `app_settings.link_previews_enabled` defaults to 0, so a row
/// another setting creates no longer turns link previews (an IP leak to
/// the linked site) on (plan C7.28).
///
/// 85: the identity store (plan D1). One encrypted database per identity;
/// the identity's record owner keypairs moved to its vault (their
/// `identity` columns and `friends.local_conversation_keypair` dropped);
/// `identity.friend_inbox_{key,keypair}` and `friends.dm_log_key` hold
/// the daemon's friend-inbox protocol until E2.1.
///
/// 86: `messages.send_state` (D22's delivery vocabulary) and the held
/// write's record slot a queued post waited on, plan D1 / C7.14.
///
/// 87: the preferences move from the plugin store into `app_settings`
/// columns and `app_input_channels` (plan D1).
///
/// 91: the daemon's friend inbox and DM logs go (plan E2.1):
/// `identity.friend_inbox_{key,keypair}` and `friends.dm_log_key` are
/// dropped; friend requests and DMs travel as `'D'` envelopes on both hosts.
///
/// 92 (plan E2.3): the 1:1 ratchet's tables `ratchet_sessions`,
/// `signal_prekeys`, `signal_identities` and `unprocessed`; the
/// envelope-store family goes with the daemon's dead call orchestration
/// (`pending_envelopes`, `seen_envelopes`, `outbound_seqs`,
/// `active_call_states`).
///
/// 93 (plan E2.4): the prekey record's `identity.prekey_record_key`,
/// `signal_prekeys.retired_at`, `prekey_lr_inits`, `prekey_rejections`
/// and `prekey_slots_used`.
///
/// 94 (audit 2026-10-20, F3): `idx_messages_dedup` covers only rows with
/// no client id (two DMs in one millisecond were one), the client-id index
/// names the sender; CHECKs on `identity.device_id` (32 hex), the 32-byte
/// keys of `signal_identities` and `unprocessed.header_dh`, and the
/// counters; `unprocessed.quarantined_at`; each vault owner entry holds its
/// record key with the keypair.
///
/// 96 (plan E2.5, D22): the outbox's `outbox`, `outbox_streams` and `received` tables.
///
/// 97 (plan E2.5): `presence_keys` and `friends.presence_epoch`/`presence_key`
/// (the friends key of profile STATUS and GAME_INFO).
///
/// 98 (plan E2.5, D24): the send log `message_send_log` and the retry
/// requests `retry_requests`/`retry_request_sends`.
///
/// 99 (plan E2.5, D17): `identity.friend_request_token`, the friend code's
/// request token.
///
/// 100 (plan E2.5, r10): `app_settings.read_receipts_enabled`, off by default.
///
/// 101 (plan E2.5, D17): `outgoing_invites.inbox_key`, the invite's
/// single-use request inbox.
///
/// 102 (plan E2.5, r10): `messages.stream_seq`, each DM's place in its stream
/// (ours at enqueue, the sender's when received), which `ReadUpTo` names.
///
/// 103 (plan E2.5, D22): `pending_messages` is gone; the friend handshake,
/// removal and lifecycle frames are `friend_handshake` outbox rows.
///
/// 104 (plan E2.5, D22): the outbox's `dht_write` stream absorbs C7.6h's
/// unsent-write table.
///
/// 105 (plan E2.5, D22): a channel post waits on its `channel_post` outbox
/// row; the held record slot columns of `messages` are gone.
///
/// 106 (plan E2.5, D23): `stream_cursors`, and `friends.participant_record_key`
/// and `peer_participant_record_key`, the dm participant records whose chains
/// carry each direction of a conversation.
///
/// 107 (plan E2.5, D23/D24): `chain_entries`, which chain seq each `dm` row
/// went to (for `retire_through`), and `messages.history_gap_through`, the
/// stored `HistoryGap` marker.
///
/// 109 (plan E2.6, G4): the SMPL group DM tables `dms`, `dm_messages` and
/// `dm_messages_fts` are gone; a group DM is `group_dms`.
///
/// 111 (plan E3.1, G1): `read_markers` (D25) replaces
/// `channel_read_state`; `messages.mentions_me`.
///
/// 113 (plan E3.1, F3h): `community_held_threads`, the threads held at
/// receipt, so a hold survives a restart.
///
/// 115 (plan E3.2, S4): `governance_ops` and `governance_heads`, the signed
/// op set and each author's head, replace `governance_entries_cache`;
/// the outbox's `governance_op` stream kind.
///
/// 116 (plan E3.2, S5): `communities.genesis_cert`, the pinned cert.
///
/// 117 (plan E3.1, FX2): `idx_messages_sender_time`, a sender's posts by
/// time, for the slowmode gate's nearest earlier post.
///
/// 118 (plan E2.5, live A/B finding 42): `outbox.peer_ack`, an ack that
/// arrived before its row's send landed.
///
/// 119 (plan E2.5, live A/B finding 71 refined): `chain_entries.appended_at`
/// is NULL while an entry is pending, `chain_entries.frame` holds its sealed
/// hint until it lands, and `friends.dm_chain_head` is the highest seq
/// assigned.
/// 120 (live finding 79): `friends.nickname` is gone; `friend_nicknames`
/// binds a nickname to the key it was given for, cleared when the
/// friendship ends.
/// 121: `app_settings.notification_content` (E5.2): what an OS notification
/// shows (name and message, name only, or nothing).
/// 122 (plan E3.2, review RV7 M1): `communities.my_role_ids` and
/// `community_members.role_ids` are dropped. Role ids, ours and every
/// member's, are the merged governance's role assignments; no row caches
/// them.
/// 123 (plan VU1): Veilid 0.5.8's table-store migration is one-way; the bump
/// resets DB, vault and Veilid storage together.
/// 124 (plan E2.5i, R1-3): `stream_applied`, the seqs of a writer's chain
/// applied past a gap, so the cursor absorbs them when the gap fills.
/// 125 (plan E3.2 S7l, D16): `outbox.kind` admits `mek_transfer`, the
/// sealed community key to one member, receipted and retried.
/// 126 (plan E3.2 S9, R5-2): `communities.removed_by_name`, the remover's
/// name as known when our removal merged, kept across teardown and restart.
/// 127 (plan E2.5, R5-5): the DM participant record is DFLT(1): the chain's
/// retire mark rides in `ChainHead`, the `RETIRE_THROUGH` subkey is gone.
/// 128 (plan E7.7.2): `app_settings.skin_id` and `sound_bank` with their
/// GLOB CHECKs, and the identity skin library `skin_packages`.
/// 129 (plan E3.2 S7l, F9-1): `outbox.state` and `messages.send_state`
/// admit `awaiting_key`, a channel post queued until its community key comes.
/// 130 (plan E3.2 S9, F9-3): `messages.failure_reason`, why our own post
/// failed (removed from the community, the key did not arrive), kept for a reload.
/// 131 (plan E3.2 S7c.6): `governance_checkpoints`, each checkpoint digest
/// a community holds with its state and, while usable, its snapshot.
/// 132 (plan E3.2 S7c.8): `governance_partial`, a community joined from a
/// checkpoint hint until its backfill ends.
/// 133 (plan E3.2 S8): `community_invites.uses` dropped.
/// 134 (plan E3.2 S8): `communities.governance_key` dropped; the community
/// id is the anchor `A`'s key (ADR 0011).
/// 135 (plan E3.2 S9): `community_roles.role_id` is the full 16-byte role
/// id as 32 hex characters, no longer a truncated integer.
/// 136 (plan E3.2 S10, D15): no SQL change; a governance op's signature
/// and hash cover its Cap'n Proto canonical form without `sig`, so ops
/// stored by older builds no longer verify.
/// 137 (plan E3.2 S10, invite proof): no SQL change; an invite's
/// `code_hash` is the public half of its join key and a `MemberJoined`
/// invite proof is that key's signature, so stored invites and joins of
/// older builds no longer verify.
/// 143 (plan E7.2 slices 7-11): `identity.bio` and `friends.bio`,
/// `identity.share_current_game`, `hidden_games` and `favourite_servers`.
/// 146 (plan E7.1 disappearing messages, E3.4 S1): `friends`/`group_dms.disappearing_seconds`,
/// `messages.expire_seconds`/`expire_at`/`timer_notice_seconds`, the send log's and
/// unprocessed rows' `client_id`, `communities.beat_seq`.
/// 147 (plan E2.6, GC1): stored group rosters (`group_dms.roster` JSON) carry each
/// member's `displayName`; rows of older builds lack it.
/// 148 (plan E3.2 LV1): `communities.my_member_record`, our own member record's key
/// stored at join/create (never read back from our merged membership).
/// 149 (plan E2.6 G4): `stream_cursors.stream_seq`, the author's outbox stream seq of the
/// entry the cursor was committed on (a retired group stretch held by hint is no gap).
/// 150 (plan E3.2 S8-H/S8-I): `communities` drops `dht_owner_keypair`, `dht_record_key`,
/// `manifest_key` and the never-advanced `lamport_clock`.
/// 151 (plan E3.2 S7s.3 + S7d-keys K1): `creator_invites` and `pending_creations`
/// (with `answered`); `governance_ops.{device_seq, device_key, signed_here}`, `governance_heads`
/// keyed by device, `governance_devices`.
/// 152 (plan E3.2 IV1b): `community_invites.url`, the link as minted.
/// 153 (round 23: HOL1 + plan E7.15 IV1/IV2 + ADR 0017 2026-11-22): `outbox.depends_on`, a
/// row's named dependency; `activity_log` and `activity_seen`; `app_settings.keep_activity_log`;
/// and the veilid-core pin moves to fork rev `bc741633`, so the DB, vault and Veilid storage
/// reset together.
/// 154 (plan E7.2 G7 + G12): `group_dms.origin_peer`, the friend whose chat the room was created
/// from, and `group_dms.favourite`, our "Chat Rooms" flag; `transfers`, each download's state and
/// resume checkpoint.
/// 155 (plan E3.4 S6): `community_members.custom_status`; the row's `display_name` and
/// `custom_status` are the member's verified card, written only from `read_card`.
/// 156 (plan E7.2 G3): `recent_servers`, the servers we visited, for the Servers view.
/// 157 (plan RV1): `route_budget`, the identity's persisted new-route budget.
/// 158 (plan RV2): `friends.reply_inbox_key`, `friends.peer_reply_inbox_key`/`peer_reply_writer`,
/// `pending_friend_requests.reply_inbox_key`/`reply_writer`: a request's reply inbox.
/// 160 (plan RV2e): `route_budget.kind`, keyed `(owner_key, kind)`: the friend code slot's
/// roll budget is persisted beside the route budget.
/// 161 (plan E7.2 G6): `group_lobbies` and `group_lobby_members`, each group's game lobbies.
/// 162 (owed item GD): `messages.reply_to_client_id`, a DM's or group DM's parent client id
/// as carried, served whether or not the parent is held.
pub const SCHEMA_VERSION: i64 = 162;

/// Opening the database failed.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    /// SQLite refused an operation; `context` says which.
    #[error("{context}: {source}")]
    Sqlite {
        context: &'static str,
        source: rusqlite::Error,
    },
    /// The key does not decrypt the identity database.
    #[error("the database key does not open this database")]
    WrongKey,
    /// The identity database was written for another schema version.
    #[error("the database is at schema version {found}, not {expected}", expected = SCHEMA_VERSION)]
    VersionMismatch { found: i64 },
    /// A file operation on the database failed.
    #[error("database file: {0}")]
    Io(std::io::Error),
}

fn sqlite(context: &'static str) -> impl FnOnce(rusqlite::Error) -> DbError {
    move |source| DbError::Sqlite { context, source }
}

/// Open an identity's encrypted database (plan D1): keyed with the
/// vault-derived `key` (SQLCipher), verified, WAL, foreign keys and
/// `secure_delete` on, `synchronous=FULL` with `fullfsync` and
/// `checkpoint_fullfsync` (macOS: `F_FULLFSYNC`, so a committed ratchet
/// step survives a power loss before the message it keys goes out), and
/// `journal_size_limit=0`, so a WAL reset truncates the file instead of
/// keeping old frames (old ratchet records, erased prekey secrets) in it,
/// and `wal_autocheckpoint=1000`, which bounds what the erase checkpoint
/// (`crate::erase`) copies. A new database gets the schema; a database at
/// another schema version is an error, never a silent reset — the layout
/// check (`crate::reset::check_layout`) already cleared a root laid out for
/// another version.
///
/// # Errors
/// [`DbError::WrongKey`] when `key` does not decrypt it,
/// [`DbError::VersionMismatch`], or the SQLite step that failed.
pub fn open(path: &Path, key: &[u8; 32]) -> Result<Db, DbError> {
    rekindle_asql::init_sqlite();
    let conn = Connection::open(path).map_err(sqlite("open database"))?;
    conn.pragma_update(None, "key", format!("x'{}'", hex_lower(key)))
        .map_err(sqlite("key database"))?;
    let tables: i64 = conn
        .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))
        .map_err(|e| match e.sqlite_error_code() {
            Some(rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::OutOfMemory) => {
                DbError::WrongKey
            }
            _ => DbError::Sqlite {
                context: "verify key",
                source: e,
            },
        })?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON; \
         PRAGMA synchronous=FULL; PRAGMA fullfsync=ON; PRAGMA checkpoint_fullfsync=ON; \
         PRAGMA journal_size_limit=0; PRAGMA wal_autocheckpoint=1000;",
    )
    .map_err(sqlite("set pragmas"))?;
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(sqlite("read schema version"))?;
    if version == 0 && tables == 0 {
        conn.execute_batch(SCHEMA).map_err(sqlite("apply schema"))?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(sqlite("set schema version"))?;
    } else if version != SCHEMA_VERSION {
        return Err(DbError::VersionMismatch { found: version });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(DbError::Io)?;
    }
    Ok(Db::from(rekindle_asql::Connection::from(conn)))
}

fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

#[cfg(test)]
mod tests {
    use rusqlite::OptionalExtension;

    use super::*;

    #[tokio::test]
    async fn an_identity_database_opens_only_with_its_key_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pk.db");
        let db = open(&path, &[7; 32]).unwrap();
        let version: i64 = db
            .call(|c| c.pragma_query_value(None, "user_version", |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        drop(db);
        assert!(matches!(open(&path, &[8; 32]), Err(DbError::WrongKey)));
        assert!(open(&path, &[7; 32]).is_ok());
        let plain = std::fs::read(&path).unwrap();
        assert!(
            !plain.starts_with(b"SQLite format 3"),
            "the file is encrypted"
        );
    }

    /// The durability settings (R3: the ratchet commits before the send):
    /// `synchronous=FULL` (2), and both full-fsync flags, which SQLite
    /// honours on macOS only but reports everywhere; and the WAL is cut to
    /// nothing at each reset (`journal_size_limit=0`).
    #[tokio::test]
    async fn a_new_database_syncs_fully() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("pk.db"), &[7; 32]).unwrap();
        let pragmas: (i64, i64, i64, i64) = db
            .call(|c| {
                let get = |name: &str| c.pragma_query_value(None, name, |r| r.get::<_, i64>(0));
                Ok((
                    get("synchronous")?,
                    get("fullfsync")?,
                    get("checkpoint_fullfsync")?,
                    get("journal_size_limit")?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(pragmas, (2, 1, 1, 0));
    }

    /// The linked SQLite is SQLCipher: `bundled-sqlcipher-vendored-openssl`
    /// must keep winning feature unification over the `bundled` that
    /// veilid-core's keyvaluedb-sqlite → async-sqlite chain asks for (plan
    /// VU1, synthesis #17). Stock SQLite answers no row to this pragma.
    #[tokio::test]
    async fn the_linked_sqlite_is_sqlcipher() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("pk.db"), &[7; 32]).unwrap();
        let version: Option<String> = db
            .call(|c| {
                c.query_row("PRAGMA cipher_version", [], |r| r.get(0))
                    .optional()
            })
            .await
            .unwrap();
        assert!(
            version.is_some_and(|v| !v.is_empty()),
            "PRAGMA cipher_version is empty: the build links stock SQLite"
        );
    }

    #[test]
    fn an_identity_database_at_another_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pk.db");
        drop(open(&path, &[7; 32]).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "key", format!("x'{}'", hex_lower(&[7; 32])))
            .unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION - 1)
            .unwrap();
        drop(conn);
        assert!(matches!(
            open(&path, &[7; 32]),
            Err(DbError::VersionMismatch { found }) if found == SCHEMA_VERSION - 1
        ));
    }
}
