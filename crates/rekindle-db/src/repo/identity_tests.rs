use super::*;
use crate::repo::fixture::{self, OWNER};

#[test]
fn an_owned_record_key_is_replaced() {
    let conn = fixture::conn();
    set_owned_record(&conn, OWNER, OwnedRecord::Profile, "k1").unwrap();
    set_owned_record(&conn, OWNER, OwnedRecord::Profile, "k2").unwrap();
    set_mailbox_key(&conn, OWNER, "mb").unwrap();
    set_owned_record(&conn, OWNER, OwnedRecord::Prekeys, "pk").unwrap();
    let (name, cols) = login_row(&conn, OWNER).unwrap().unwrap();
    assert_eq!(name, "me");
    assert_eq!(cols.profile.as_deref(), Some("k2"));
    assert_eq!(cols.mailbox.as_deref(), Some("mb"));
    assert_eq!(cols.prekeys.as_deref(), Some("pk"));
    assert_eq!(cols.account, None);
    assert_eq!(login_row(&conn, "ghost").unwrap(), None);
}

#[test]
fn the_avatar_round_trips() {
    let conn = fixture::conn();
    assert_eq!(avatar(&conn, OWNER).unwrap(), None);
    set_avatar(&conn, OWNER, &[9]).unwrap();
    assert_eq!(avatar(&conn, OWNER).unwrap(), Some(vec![9]));
}

#[test]
fn rekey_moves_the_identity_and_everything_it_owns() {
    let mut conn = fixture::conn();
    crate::repo::friends::insert_accepted(&conn, OWNER, "bob", "Bob", 1).unwrap();
    set_mailbox_key(&conn, OWNER, "mb").unwrap();
    rekey(&mut conn, OWNER, "new-pk", None).unwrap();
    assert_eq!(login_row(&conn, OWNER).unwrap(), None);
    // The old identity's mailbox does not carry over.
    let (_, cols) = login_row(&conn, "new-pk").unwrap().unwrap();
    assert_eq!(cols.mailbox, None);
    assert_eq!(
        crate::repo::friends::load_all(&conn, "new-pk")
            .unwrap()
            .len(),
        1
    );
    assert!(crate::repo::friends::load_all(&conn, OWNER)
        .unwrap()
        .is_empty());
    let violations: i64 = conn
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0);
}

/// Our own rotation (audit 2026-10-20 R-2, R-3): our messages move to
/// the new key, verifications are cleared, open invites are cancelled,
/// requests to or from the old key go, and so do frames queued under
/// the deleted sessions; sealed queued kinds stay.
#[test]
fn rekey_settles_what_the_old_key_was_addressed_as() {
    let mut conn = fixture::conn();
    crate::repo::friends::insert_accepted(&conn, OWNER, "bob", "Bob", 1).unwrap();
    conn.execute_batch(&format!(
        "INSERT INTO friends (owner_key, public_key, added_at, friendship_state) \
               VALUES ('{OWNER}', 'carol', 1, 'pending_out'); \
             INSERT INTO pending_friend_requests \
               (owner_key, public_key, display_name, received_at, reply_inbox_key, reply_writer) \
               VALUES ('{OWNER}', 'dave', 'Dave', 1, 'VLD0:reply', 'VLD0:writer'); \
             INSERT INTO messages (owner_key, conversation_id, conversation_type, sender_key, body, timestamp) \
               VALUES ('{OWNER}', 'bob', 'dm', '{OWNER}', 'mine', 1), \
                      ('{OWNER}', 'bob', 'dm', 'bob', 'theirs', 2); \
             INSERT INTO signal_identities (owner_key, peer_identity, identity_key, verified, verified_at) \
               VALUES ('{OWNER}', 'bob', zeroblob(32), 1, 5);"
    ))
    .unwrap();
    for (stream, id, payload) in [
        (
            "friend_handshake:bob",
            "w",
            &br#"{"kind":"wire","datagram":"AA=="}"#[..],
        ),
        ("friend_handshake:bob", "s", br#"{"kind":"sealed"}"#),
        ("friend_handshake:carol", "c", br#"{"kind":"sealed"}"#),
    ] {
        crate::repo::outbox::queue_for_test(&conn, OWNER, stream, id, payload);
    }
    insert_invite(&conn, "i1");
    rekey(&mut conn, OWNER, "new-pk", None).unwrap();
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(
        count("SELECT count(*) FROM messages WHERE sender_key = 'new-pk' AND body = 'mine'"),
        1
    );
    assert_eq!(
        count("SELECT count(*) FROM messages WHERE sender_key = 'bob'"),
        1
    );
    assert_eq!(
        count("SELECT count(*) FROM signal_identities WHERE verified = 1"),
        0
    );
    assert_eq!(
        count("SELECT count(*) FROM outgoing_invites WHERE status = 'cancelled'"),
        1
    );
    assert_eq!(count("SELECT count(*) FROM pending_friend_requests"), 0);
    assert_eq!(count("SELECT count(*) FROM friends"), 1);
    assert_eq!(
        count("SELECT count(*) FROM outbox WHERE owner_key = 'new-pk' AND client_id = 's'"),
        1
    );
}

fn insert_invite(conn: &Connection, id: &str) {
    crate::repo::outgoing_invites::insert(conn, OWNER, id, "u", "inbox", 1).unwrap();
}

/// The rotation stores the new identity's minted set under the new
/// key, after the old identity's prekeys and their bookkeeping go, in
/// the same transaction.
#[test]
fn rekey_replaces_the_prekeys_with_the_new_set() {
    use rekindle_crypto::signal::prekey_record::mint_set;
    let mut conn = fixture::conn();
    let old = rekindle_crypto::Identity::generate();
    let rng = rekindle_utils::random::OsRandom;
    crate::prekeys::store_set_rows(&conn, OWNER, &mint_set(&old, &rng, 1)).unwrap();
    conn.execute(
        "INSERT INTO prekey_lr_inits VALUES (?1, 1, 1, zeroblob(32), 0)",
        params![OWNER],
    )
    .unwrap();
    let new = rekindle_crypto::Identity::generate();
    let set = mint_set(&new, &rng, 2);
    let head = set.head.spk_id;
    rekey(&mut conn, OWNER, "new-pk", Some(&set)).unwrap();
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(
        count("SELECT count(*) FROM signal_prekeys WHERE owner_key = 'new-pk'"),
        202
    );
    assert_eq!(count("SELECT count(*) FROM signal_prekeys"), 202);
    assert_eq!(count("SELECT count(*) FROM prekey_lr_inits"), 0);
    let spk: u32 = conn
        .query_row(
            "SELECT id FROM signal_prekeys WHERE kind = 'spk' AND owner_key = 'new-pk'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(spk, head);
}

const DEV_A: &str = "0123456789abcdef0123456789abcdef";
const DEV_B: &str = "fedcba9876543210fedcba9876543210";

#[test]
fn the_device_id_is_minted_once() {
    let conn = fixture::conn();
    assert_eq!(ensure_device_id(&conn, OWNER, DEV_A).unwrap(), DEV_A);
    assert_eq!(ensure_device_id(&conn, OWNER, DEV_B).unwrap(), DEV_A);
    let (_, cols) = login_row(&conn, OWNER).unwrap().unwrap();
    assert_eq!(cols.device_id.as_deref(), Some(DEV_A));
}

/// Audit D-9: a device id is 32 lowercase hex characters.
#[test]
fn a_malformed_device_id_is_refused() {
    let conn = fixture::conn();
    for bad in [
        "",
        "aa",
        "0123456789ABCDEF0123456789ABCDEF",
        "g123456789abcdef0123456789abcdef",
    ] {
        assert!(ensure_device_id(&conn, OWNER, bad).is_err(), "{bad:?}");
    }
}

/// Audit D-6: the personal sync record names the device id the
/// own-records stage minted; it never replaces it.
#[test]
fn personal_sync_keeps_the_device_id() {
    let conn = fixture::conn();
    assert_eq!(personal_sync(&conn, OWNER).unwrap(), None);
    ensure_device_id(&conn, OWNER, DEV_A).unwrap();
    assert_eq!(set_personal_sync(&conn, OWNER, "rk", DEV_B).unwrap(), DEV_A);
    assert_eq!(
        personal_sync(&conn, OWNER).unwrap(),
        Some(("rk".into(), DEV_A.into()))
    );
    let (_, cols) = login_row(&conn, OWNER).unwrap().unwrap();
    assert_eq!(cols.device_id.as_deref(), Some(DEV_A));
}

#[test]
fn personal_sync_mints_the_device_id_when_none_is_held() {
    let conn = fixture::conn();
    assert_eq!(set_personal_sync(&conn, OWNER, "rk", DEV_B).unwrap(), DEV_B);
    assert_eq!(ensure_device_id(&conn, OWNER, DEV_A).unwrap(), DEV_B);
}
