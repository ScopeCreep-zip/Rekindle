use super::*;

fn entry(key: &str) -> OwnerEntry {
    OwnerEntry {
        record_key: key.into(),
        keypair: format!("kp-{key}"),
    }
}

fn stored(db_key: Option<&str>, entry_key: Option<&str>) -> StoredRecord {
    StoredRecord {
        db_key: db_key.map(Into::into),
        entry: entry_key.map(entry),
    }
}

/// Audit D-5: a crash between the vault write and the row write (a
/// block's rotation, or a first create) is found and the row follows
/// the vault; the row's record is never opened with another record's
/// keypair.
#[test]
fn the_unlock_follows_the_vault_after_an_interrupted_write() {
    let agreed = stored(Some("k1"), Some("k1"));
    assert_eq!(plan(&agreed), Plan::Reopen(&entry("k1")));
    assert_eq!(agreed.matching_keypair(), Some("kp-k1"));
    let rotated = stored(Some("old"), Some("new"));
    assert_eq!(plan(&rotated), Plan::Repair(&entry("new")));
    assert_eq!(rotated.matching_keypair(), None);
    let first = stored(None, Some("new"));
    assert_eq!(plan(&first), Plan::Repair(&entry("new")));
    assert_eq!(plan(&stored(Some("k1"), None)), Plan::Replace);
    assert_eq!(plan(&stored(None, None)), Plan::Create);
}

async fn unlocked() -> (super::super::fixture::Fixture, String) {
    let fixture = super::super::fixture::Fixture::new();
    let dir = fixture.root.state.clone();
    let (vault, key) =
        VaultStore::create(&dir.join("pk.vault"), rekindle_test_fixtures::PASSPHRASE).unwrap();
    *fixture.vault.lock() = Some(vault);
    let db = rekindle_db::open(&dir.join("pk.db"), key.as_bytes()).unwrap();
    db.call(|c| {
        rekindle_db::repo::identity::insert(c, "pk", "me", 0, "aaaaaaaaaaaaaaaaaaaaaaaaaa")
    })
    .await
    .unwrap();
    fixture.db.set(db);
    (fixture, "pk".to_string())
}

fn row_key(stored: &StoredRecord) -> Option<&str> {
    stored.db_key.as_deref()
}

async fn load(fixture: &super::super::fixture::Fixture) -> StoredRecord {
    let db = fixture.db.current().unwrap();
    let key = db
        .call(|c| rekindle_db::repo::identity::login_row(c, "pk"))
        .await
        .unwrap()
        .unwrap()
        .1
        .profile;
    let guard = fixture.vault.lock();
    StoredRecord::load(guard.as_ref().unwrap(), OwnedRecord::Profile, key).unwrap()
}

/// A save writes the vault entry and the row; one whose row write fails
/// leaves the previous entry in the vault.
#[tokio::test]
async fn a_save_writes_both_or_restores_the_vault() {
    let (fixture, pk) = unlocked().await;
    let owner = Owner {
        vault: &fixture.vault,
        db: &fixture.db,
        public_key: &pk,
    };
    owner
        .save(OwnedRecord::Profile, "k1", "kp-k1")
        .await
        .unwrap();
    let stored = load(&fixture).await;
    assert_eq!(row_key(&stored), Some("k1"));
    assert_eq!(stored.entry, Some(entry("k1")));

    // No database to write the row to: the vault is put back.
    let closed = DbHandle::default();
    let failing = Owner {
        vault: &fixture.vault,
        db: &closed,
        public_key: &pk,
    };
    assert!(failing
        .save(OwnedRecord::Profile, "k2", "kp-k2")
        .await
        .is_err());
    let stored = load(&fixture).await;
    assert_eq!(row_key(&stored), Some("k1"));
    assert_eq!(stored.entry, Some(entry("k1")));
}
