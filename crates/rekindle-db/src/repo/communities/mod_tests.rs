use super::*;
use crate::repo::fixture::{self, OWNER};

fn community<'a>(id: &'a str, name: &'a str) -> NewCommunity<'a> {
    NewCommunity {
        id,
        name,
        joined_at: 0,
        my_pseudonym_key: "me",
        mek_generation: 3,
        member_registry_key: Some("registry"),
        my_subkey_index: Some(4),
    }
}

#[test]
fn segment_and_mek_generation_are_set() {
    let conn = fixture::conn();
    insert(&conn, OWNER, &community("c1", "One")).unwrap();
    set(&conn, OWNER, "c1", Column::MySegmentIndex, 2).unwrap();
    set(&conn, OWNER, "c1", Column::MekGeneration, 9).unwrap();
    let row = &load_all(&conn, OWNER).unwrap()[0];
    assert_eq!(row.my_segment_index, Some(2));
    assert_eq!(row.mek_generation, 9);
}

#[test]
fn genesis_cert_is_stored_and_read() {
    let conn = fixture::conn();
    insert(&conn, OWNER, &community("c1", "One")).unwrap();
    assert_eq!(genesis_cert(&conn, OWNER, "c1").unwrap(), None);
    set(&conn, OWNER, "c1", Column::GenesisCert, vec![1u8, 2, 3]).unwrap();
    assert_eq!(
        genesis_cert(&conn, OWNER, "c1").unwrap(),
        Some(vec![1, 2, 3])
    );
    assert_eq!(genesis_cert(&conn, OWNER, "absent").unwrap(), None);
}

/// After a restart the message clock resumes from the community's
/// highest stored channel `lamport_ts` (only its own channels, never
/// DMs).
#[test]
fn message_clock_reloads_from_storage() {
    let conn = fixture::conn();
    insert(&conn, OWNER, &community("c1", "One")).unwrap();
    insert(&conn, OWNER, &community("c2", "One")).unwrap();
    conn.execute_batch(&format!(
        "INSERT INTO channels (owner_key, id, community_id, name, channel_type) \
               VALUES ('{OWNER}', 'ch1', 'c1', 'general', 'text'), \
                      ('{OWNER}', 'ch2', 'c2', 'general', 'text'); \
             INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp, lamport_ts) \
               VALUES ('{OWNER}', 'ch1', 'channel', 's', 'a', 1, 900), \
                      ('{OWNER}', 'ch1', 'channel', 's', 'b', 2, 1500), \
                      ('{OWNER}', 'ch2', 'channel', 's', 'c', 3, 7), \
                      ('{OWNER}', 'ch1', 'dm', 's', 'not a channel', 4, 99999);"
    ))
    .unwrap();
    let rows = load_all(&conn, OWNER).unwrap();
    let clock = |id: &str| rows.iter().find(|r| r.id == id).unwrap().message_clock;
    assert_eq!(clock("c1"), 1500);
    assert_eq!(clock("c2"), 7);
}

#[test]
fn a_joined_community_is_not_overwritten_and_columns_set_individually() {
    let conn = fixture::conn();
    insert_if_absent(&conn, OWNER, &community("c1", "One")).unwrap();
    insert_if_absent(&conn, OWNER, &community("c1", "Other")).unwrap();
    set(&conn, OWNER, "c1", Column::Description, "about").unwrap();
    set(&conn, OWNER, "c1", Column::MySubkeyIndex, 8_i64).unwrap();
    let row = &load_all(&conn, OWNER).unwrap()[0];
    assert_eq!(row.name, "One");
    assert_eq!(row.description.as_deref(), Some("about"));
    assert_eq!(
        (
            row.my_subkey_index,
            row.my_segment_index,
            row.mek_generation
        ),
        (Some(8), Some(0), 3)
    );
    delete(&conn, OWNER, "c1").unwrap();
    assert!(load_all(&conn, OWNER).unwrap().is_empty());
}

#[test]
fn deleting_a_community_deletes_every_row_that_belongs_to_it() {
    let conn = fixture::conn();
    fixture::community(&conn, "c1");
    fixture::community(&conn, "c2");
    let per_community = [
        "INSERT INTO community_roles (owner_key, community_id, role_id, name) VALUES (?1, ?2, '01010101010101010101010101010101', 'r')",
        "INSERT INTO community_members (owner_key, community_id, pseudonym_key, joined_at) VALUES (?1, ?2, 'p', 0)",
        "INSERT INTO event_rsvps (owner_key, community_id, event_id, pseudonym_key, status) VALUES (?1, ?2, 'e', 'p', 'going')",
        "INSERT INTO read_markers (owner_key, scope_kind, community_id, channel_id, last_read_seq, updated_at) VALUES (?1, 'channel', ?2, 'ch', 0, 0)",
        "INSERT INTO governance_ops (owner_key, community_id, op_hash, author, device_key, device_seq, lamport, issued_at, op, state, signed_here) VALUES (?1, ?2, zeroblob(32), 'a', 'a', 0, 1, 0, x'00', 'applied', 0)",
        "INSERT INTO governance_heads (owner_key, community_id, device_key, author, member_record, head_seq, frontier_hash) VALUES (?1, ?2, 'a', 'a', 'm', 0, zeroblob(32))",
        "INSERT INTO governance_devices (owner_key, community_id, member, device_key, incarnation, device_record, state) VALUES (?1, ?2, 'p', 'd', 0, 'm', 'effective')",
        "INSERT INTO governance_checkpoints (owner_key, community_id, height, round, digest, op_hash, dir_size, state) VALUES (?1, ?2, 1, 0, zeroblob(32), zeroblob(32), 0, 'usable')",
        "INSERT INTO governance_partial (owner_key, community_id, height, digest, state_root) VALUES (?1, ?2, 1, zeroblob(32), zeroblob(32))",
    ];
    for sql in per_community {
        for id in ["c1", "c2"] {
            conn.execute(sql, params![OWNER, id]).unwrap();
        }
    }
    delete(&conn, OWNER, "c1").unwrap();
    for table in [
        "community_roles",
        "community_members",
        "event_rsvps",
        "read_markers",
        "governance_ops",
        "governance_heads",
        "governance_devices",
        "governance_checkpoints",
        "governance_partial",
    ] {
        let left: Vec<String> = conn
            .prepare(&format!("SELECT community_id FROM {table}"))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(left, ["c2"], "{table}");
    }
}

#[test]
fn a_row_for_a_community_we_do_not_hold_is_refused() {
    let conn = fixture::conn();
    let err = conn.execute(
        "INSERT INTO community_roles (owner_key, community_id, role_id, name) VALUES (?1, 'gone', '01010101010101010101010101010101', 'r')",
        params![OWNER],
    );
    assert!(err.is_err());
}

#[test]
fn an_rsvp_status_outside_the_vocabulary_is_refused() {
    let conn = fixture::conn();
    fixture::community(&conn, "c1");
    let err = conn.execute(
        "INSERT INTO event_rsvps (owner_key, community_id, event_id, pseudonym_key, status) \
             VALUES (?1, 'c1', 'e', 'p', 'none')",
        params![OWNER],
    );
    assert!(err.is_err());
}

/// A re-join over the row a removal kept: the membership columns are
/// the new ones, the removal's name goes, the kept history stays.
#[test]
fn a_membership_over_kept_history_replaces_its_membership() {
    let conn = fixture::conn();
    upsert_membership(&conn, OWNER, &community("c1", "One")).unwrap();
    set(&conn, OWNER, "c1", Column::Description, "kept").unwrap();
    set(&conn, OWNER, "c1", Column::RemovedByName, "Ann").unwrap();
    let rejoined = NewCommunity {
        mek_generation: 0,
        my_subkey_index: Some(7),
        ..community("c1", "One")
    };
    upsert_membership(&conn, OWNER, &rejoined).unwrap();
    let rows = load_all(&conn, OWNER).unwrap();
    let [row] = rows.as_slice() else {
        panic!("one row: {rows:?}");
    };
    assert_eq!(row.removed_by_name, None);
    assert_eq!(row.my_subkey_index, Some(7));
    assert_eq!(row.mek_generation, 0);
    assert_eq!(row.description.as_deref(), Some("kept"));
}
