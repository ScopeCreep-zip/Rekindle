//! Every enum and JSON column refuses a value outside its vocabulary
//! (plan D2): a corrupt value cannot be written, and the readers parse
//! strictly for one that is (`row::get_enum`, `row::get_json`).

use rusqlite::{params, Connection};

use crate::repo::fixture::{self, OWNER};

/// One valid row in every table with an enum or JSON column.
fn seeded() -> Connection {
    let conn = fixture::conn();
    fixture::community(&conn, "c");
    for sql in [
        "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
         VALUES (?1, 'f', 0, 'accepted')",
        "INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, \
         timestamp, attachment_json) VALUES (?1, 'ch', 'channel', 's', 'b', 0, '{}')",
        "INSERT INTO channels (owner_key, id, community_id, name, channel_type) \
         VALUES (?1, 'ch', 'c', 'general', 'text')",
        "INSERT INTO community_members (owner_key, community_id, pseudonym_key, joined_at) \
         VALUES (?1, 'c', 'p', 0)",
        "INSERT INTO channel_overwrites (owner_key, community_id, channel_id, target_type, target_id) \
         VALUES (?1, 'c', 'ch', 'role', '1')",
        "INSERT INTO audit_entries (owner_key, cursor, prev_mac, mac, payload_json) \
         VALUES (?1, 0, x'00', x'00', '{}')",
        "INSERT INTO missed_calls (call_id, owner_key, peer_key, kind, expired_at) \
         VALUES ('call', ?1, 'peer', 0, 0)",
        "INSERT INTO notification_preferences (owner_key, community_id) VALUES (?1, 'c')",
        "INSERT INTO outgoing_invites (owner_key, invite_id, inbox_key, created_at, expires_at) \
         VALUES (?1, 'i', 'k', 0, 0)",
        "INSERT INTO group_dms (owner_key, group_id, name, roster, epoch, created_at) \
         VALUES (?1, '00000000000000000000000000000000', 'g', '[]', 0, 0)",
        "INSERT INTO community_events (owner_key, community_id, id, title, creator_pseudonym, \
         start_time, created_at, recurrence_json, location_json) \
         VALUES (?1, 'c', 'e', 't', 'p', 0, 0, '{}', '{}')",
        "INSERT INTO event_rsvps (owner_key, community_id, event_id, pseudonym_key, status) \
         VALUES (?1, 'c', 'e', 'p', 'going')",
        "INSERT INTO message_delivery (owner_key, message_id, community_id, recipient_pseudonym) \
         VALUES (?1, 'm', 'c', 'p')",
        "INSERT INTO push_relay_registrations (owner_key, relay_pseudonym, device_push_token, \
         platform, record_keys_json, created_at) VALUES (?1, 'r', 't', 'self', '[]', 0)",
        "INSERT INTO voice_session_events (owner_key, community_id, channel_id, member_pseudonym, \
         event_type, occurred_at) VALUES (?1, 'c', 'ch', 'p', 'join', 0)",
        "INSERT INTO governance_ops (owner_key, community_id, op_hash, author, author_seq, \
         lamport, issued_at, op, state) VALUES (?1, 'c', zeroblob(32), 'a', 0, 1, 0, x'00', \
         'applied')",
        "INSERT INTO governance_heads (owner_key, community_id, author, member_record, head_seq, \
         frontier_hash) VALUES (?1, 'c', 'a', 'm', 0, zeroblob(32))",
        "INSERT INTO signal_prekeys (owner_key, id, kind, secret, public, created_at, \
         published_subkey) VALUES (?1, 1, 'spk', x'00', x'00', 0, 0)",
        "INSERT INTO prekey_slots_used (owner_key, peer, device_id, opk_id, used_at) \
         VALUES (?1, 'p', '', 1, 0)",
        "INSERT INTO prekey_rejections (owner_key, sender, base_key_hash, sent_at) \
         VALUES (?1, 's', zeroblob(32), 0)",
        "INSERT INTO prekey_lr_inits (owner_key, lr_id, spk_id, base_key, seen_at) \
         VALUES (?1, 1, 1, zeroblob(32), 0)",
        "INSERT INTO signal_identities (owner_key, peer_identity, identity_key) \
         VALUES (?1, 'p', zeroblob(32))",
        "INSERT INTO unprocessed (owner_key, peer_identity, header_dh, header_n, type_id, \
         plaintext, received_at) VALUES (?1, 'p', zeroblob(32), 0, 256, x'00', 0)",
        "INSERT INTO read_markers (owner_key, scope_kind, community_id, channel_id, \
         last_read_seq, updated_at) VALUES (?1, 'channel', 'c', 'ch', 0, 0)",
        "INSERT INTO read_markers (owner_key, scope_kind, peer_key, last_read_seq, updated_at) \
         VALUES (?1, 'dm', 'f', 0, 0)",
        "INSERT INTO community_blocks (owner_key, community_id, pseudonym, blocked_at) \
         VALUES (?1, 'c', 'p', 0)",
        "INSERT INTO community_held_threads (owner_key, community_id, thread_id, held_at) \
         VALUES (?1, 'c', '0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a0a', 0)",
    ] {
        conn.execute(sql, params![OWNER]).unwrap_or_else(|e| panic!("{sql}: {e}"));
    }
    conn
}

/// `(table, column, a value outside its vocabulary)`.
const OUT_OF_VOCABULARY: &[(&str, &str, &str)] = &[
    ("friends", "friendship_state", "'active'"),
    ("messages", "conversation_type", "'group'"),
    ("messages", "send_state", "'sending'"),
    ("messages", "attachment_json", "'not json'"),
    ("communities", "my_role_ids", "'[0'"),
    ("communities", "presence_policy", "'{'"),
    ("channels", "channel_type", "'unknown'"),
    ("community_members", "role_ids", "'0,1'"),
    ("community_members", "badges", "'early'"),
    ("channel_overwrites", "target_type", "'everyone'"),
    ("audit_entries", "payload_json", "'}'"),
    ("missed_calls", "kind", "2"),
    ("notification_preferences", "level", "3"),
    ("outgoing_invites", "status", "'lost'"),
    ("community_events", "status", "'postponed'"),
    ("community_events", "recurrence_json", "'weekly'"),
    ("community_events", "location_json", "'here'"),
    ("event_rsvps", "status", "'none'"),
    ("message_delivery", "status", "'queued'"),
    ("group_dms", "roster", "'{}'"),
    ("push_relay_registrations", "platform", "'sms'"),
    ("push_relay_registrations", "record_keys_json", "'k'"),
    ("voice_session_events", "event_type", "'mute'"),
    ("governance_ops", "state", "'merged'"),
    ("governance_ops", "op_hash", "x'00'"),
    ("governance_ops", "author_seq", "-1"),
    ("governance_ops", "lamport", "-1"),
    ("governance_ops", "issued_at", "-1"),
    ("governance_heads", "head_seq", "-1"),
    ("governance_heads", "frontier_hash", "zeroblob(31)"),
    ("signal_prekeys", "kind", "'otpk'"),
    ("signal_prekeys", "id", "0"),
    ("signal_prekeys", "id", "2147483648"),
    ("signal_prekeys", "published_subkey", "101"),
    ("prekey_slots_used", "opk_id", "0"),
    ("prekey_rejections", "base_key_hash", "x'00'"),
    ("prekey_lr_inits", "lr_id", "2147483648"),
    ("prekey_lr_inits", "spk_id", "0"),
    ("prekey_lr_inits", "base_key", "x'00'"),
    ("signal_identities", "verified", "2"),
    ("signal_identities", "identity_key", "x'00'"),
    (
        "signal_identities",
        "identity_key",
        "'0123456789abcdef0123456789abcdef'",
    ),
    ("signal_identities", "changed_from_key", "zeroblob(31)"),
    ("signal_identities", "verified_at", "-1"),
    ("signal_identities", "changed_at", "-1"),
    ("unprocessed", "type_id", "65536"),
    ("unprocessed", "header_dh", "x'00'"),
    ("unprocessed", "header_n", "-1"),
    ("unprocessed", "attempts", "-1"),
    ("unprocessed", "quarantined_at", "-1"),
    ("identity", "device_id", "''"),
    (
        "identity",
        "device_id",
        "'0123456789ABCDEF0123456789ABCDEF'",
    ),
    ("identity", "device_id", "'0123456789abcdef'"),
    ("messages", "mentions_me", "2"),
    ("read_markers", "scope_kind", "'group'"),
    ("read_markers", "last_read_seq", "-1"),
    ("read_markers", "peer_key", "'f'"),
    ("read_markers", "community_id", "'c'"),
    ("read_markers", "channel_id", "NULL"),
    ("community_blocks", "pseudonym", "''"),
    ("community_blocks", "pseudonym", "'P'"),
    ("community_blocks", "blocked_at", "-1"),
    ("community_held_threads", "thread_id", "'t'"),
    (
        "community_held_threads",
        "thread_id",
        "'0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A0A'",
    ),
    ("community_held_threads", "held_at", "-1"),
];

#[test]
fn every_enum_and_json_column_refuses_a_value_outside_its_vocabulary() {
    let conn = seeded();
    for (table, column, value) in OUT_OF_VOCABULARY {
        let err = conn
            .execute(&format!("UPDATE {table} SET {column} = {value}"), [])
            .expect_err(&format!("{table}.{column} took {value}"));
        assert!(
            err.to_string().contains("CHECK constraint failed"),
            "{table}.{column}: {err}"
        );
    }
}

/// A corrupt value that reaches the file anyway (written past the CHECKs)
/// is a read error naming its column, never a default.
#[test]
fn a_corrupt_json_or_enum_value_is_a_read_error() {
    let conn = seeded();
    conn.pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    conn.execute("UPDATE communities SET my_role_ids = 'x'", [])
        .unwrap();
    assert!(crate::repo::communities::load_all(&conn, OWNER).is_err());

    conn.execute(
        "UPDATE communities SET my_role_ids = '[0]', presence_policy = '{\"no\": 1'",
        [],
    )
    .unwrap();
    let err = crate::repo::communities::load_all(&conn, OWNER)
        .unwrap_err()
        .to_string();
    assert!(err.contains("presence_policy"), "{err}");

    conn.execute("UPDATE communities SET presence_policy = NULL", [])
        .unwrap();
    conn.execute("UPDATE community_members SET badges = 'early'", [])
        .unwrap();
    let err = crate::repo::members::load_profiles(&conn, OWNER)
        .unwrap_err()
        .to_string();
    assert!(err.contains("badges"), "{err}");

    conn.execute(
        "UPDATE community_members SET badges = '[]', role_ids = '0'",
        [],
    )
    .unwrap();
    assert!(crate::repo::members::roster(&conn, OWNER, "c").is_err());
}
