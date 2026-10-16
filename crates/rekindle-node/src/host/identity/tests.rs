use super::*;

struct Fixture {
    _dir: tempfile::TempDir,
    root: DataRoot,
    db: Arc<DbHandle>,
    vault: VaultHandle,
    attempts: AttemptLimiter<String>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let root = DataRoot {
            config: dir.path().join("config"),
            logs: dir.path().join("logs"),
            state: data.join("state"),
            data,
        };
        std::fs::create_dir_all(&root.state).unwrap();
        Self {
            _dir: dir,
            root,
            db: Arc::new(DbHandle::default()),
            vault: rekindle_vault::new_handle(),
            attempts: AttemptLimiter::default(),
        }
    }

    fn stores(&self) -> Stores<'_> {
        Stores {
            db: &self.db,
            vault: &self.vault,
            attempts: &self.attempts,
        }
    }
}

#[tokio::test]
async fn create_lock_unlock_round_trips_and_lists() {
    let f = Fixture::new();
    let made = create(&f.root, &f.stores(), Some("Alice"), "pass", 1)
        .await
        .unwrap();
    assert_eq!(made.display_name, "Alice");
    assert!(f.db.current().is_ok());
    assert!(f.vault.lock().is_some());
    assert!(made.signal.current_bundle().is_ok());
    assert!(f.root.identity_db(&made.public_key).exists());
    assert!(f.root.identity_vault(&made.public_key).exists());
    let listed = list(&f.root).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].public_key, made.public_key);

    let pk = made.public_key.clone();
    drop(made);
    lock(&f.stores()).await;
    assert!(f.db.current().is_err());
    assert!(f.vault.lock().is_none());

    let again = unlock(&f.root, &f.stores(), &pk, "pass", 2).await.unwrap();
    assert_eq!(again.public_key, pk);
    assert_eq!(again.display_name, "Alice");
}

#[tokio::test]
async fn a_default_name_is_given_when_none_is() {
    let f = Fixture::new();
    let made = create(&f.root, &f.stores(), Some("  "), "pass", 1)
        .await
        .unwrap();
    assert!(made.display_name.starts_with("User_"));
}

#[tokio::test]
async fn wrong_passphrases_lock_the_identity() {
    let f = Fixture::new();
    let made = create(&f.root, &f.stores(), None, "right", 1)
        .await
        .unwrap();
    let pk = made.public_key.clone();
    drop(made);
    lock(&f.stores()).await;
    for _ in 0..5 {
        assert!(matches!(
            unlock(&f.root, &f.stores(), &pk, "wrong", 10).await,
            Err(IdentityError::WrongPassphrase)
        ));
    }
    assert!(matches!(
        unlock(&f.root, &f.stores(), &pk, "right", 20).await,
        Err(IdentityError::Locked { .. })
    ));
    assert!(unlock(&f.root, &f.stores(), &pk, "right", 60_010)
        .await
        .is_ok());
}

#[tokio::test]
async fn unknown_identities_are_not_found_and_delete_removes_everything() {
    let f = Fixture::new();
    assert!(matches!(
        unlock(&f.root, &f.stores(), &"ab".repeat(32), "p", 1).await,
        Err(IdentityError::NotFound)
    ));
    let made = create(&f.root, &f.stores(), None, "pass", 1).await.unwrap();
    let pk = made.public_key.clone();
    drop(made);
    lock(&f.stores()).await;
    assert!(matches!(
        delete(&f.root, &f.attempts, &pk, "wrong", 2),
        Err(IdentityError::WrongPassphrase)
    ));
    assert!(
        f.root.identity_vault(&pk).exists(),
        "a wrong passphrase deletes nothing"
    );
    delete(&f.root, &f.attempts, &pk, "pass", 3).unwrap();
    assert!(!f.root.identity_vault(&pk).exists());
    assert!(!f.root.identity_db(&pk).exists());
    assert!(list(&f.root).unwrap().is_empty());
    assert!(matches!(
        delete(&f.root, &f.attempts, &pk, "pass", 4),
        Err(IdentityError::NotFound)
    ));
}

#[tokio::test]
async fn a_rename_reaches_the_registry() {
    let f = Fixture::new();
    let made = create(&f.root, &f.stores(), Some("Old"), "pass", 1)
        .await
        .unwrap();
    rename(&f.root, &made.public_key, "New").unwrap();
    assert_eq!(list(&f.root).unwrap()[0].display_name, "New");
    assert!(matches!(
        rename(&f.root, "missing", "x"),
        Err(IdentityError::NotFound)
    ));
}

#[tokio::test]
async fn an_exported_identity_imports_on_another_root_and_unlocks() {
    let a = Fixture::new();
    let made = create(&a.root, &a.stores(), Some("Mover"), "pass", 1)
        .await
        .unwrap();
    let pk = made.public_key.clone();
    drop(made);
    // Exported while unlocked: read through the live vault.
    let archive = export(&a.root, &a.stores(), &pk, "pass", 2).unwrap();
    assert!(matches!(
        export(&a.root, &a.stores(), &pk, "wrong", 3),
        Err(IdentityError::WrongPassphrase)
    ));

    let b = Fixture::new();
    assert!(matches!(
        import(&b.root, &b.attempts, &archive, "wrong", 4).await,
        Err(IdentityError::WrongPassphrase)
    ));
    assert!(
        !b.root.identity_vault(&pk).exists(),
        "a failed import leaves nothing"
    );
    let entry = import(&b.root, &b.attempts, &archive, "pass", 5)
        .await
        .unwrap();
    assert_eq!(entry.display_name, "Mover");
    assert!(matches!(
        import(&b.root, &b.attempts, &archive, "pass", 6).await,
        Err(IdentityError::Exists)
    ));
    let unlocked = unlock(&b.root, &b.stores(), &pk, "pass", 7).await.unwrap();
    assert_eq!(unlocked.display_name, "Mover");
}

#[tokio::test]
async fn an_archive_naming_a_path_is_refused() {
    let a = Fixture::new();
    let made = create(&a.root, &a.stores(), None, "pass", 1).await.unwrap();
    let pk = made.public_key.clone();
    drop(made);
    let bytes = export(&a.root, &a.stores(), &pk, "pass", 2).unwrap();
    let mut parsed = archive::Archive::decode(&bytes).unwrap();
    parsed.entry.public_key = "../escape".into();
    let b = Fixture::new();
    assert!(matches!(
        import(&b.root, &b.attempts, &parsed.encode().unwrap(), "pass", 3).await,
        Err(IdentityError::Corrupt(_))
    ));
}
