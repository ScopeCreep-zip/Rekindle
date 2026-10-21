use super::*;

use crate::host::fixture::Fixture;

#[tokio::test]
async fn create_lock_unlock_round_trips_and_lists() {
    let f = Fixture::new();
    let made = create(&f.root, &f.stores(), Some("Alice"), "pass", 1)
        .await
        .unwrap();
    assert_eq!(made.display_name, "Alice");
    assert!(f.db.current().is_ok());
    assert!(f.vault.lock().is_some());
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
    let cached = f.root.identity_files(&pk).join("scope").join("attachment");
    std::fs::create_dir_all(&cached).unwrap();
    std::fs::write(cached.join("0.bin"), b"chunk").unwrap();
    delete(&f.root, &f.attempts, &pk, "pass", 3).unwrap();
    assert!(!f.root.identity_vault(&pk).exists());
    assert!(!f.root.identity_db(&pk).exists());
    assert!(
        !f.root.identity_files(&pk).exists(),
        "the file cache goes too"
    );
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

#[tokio::test]
async fn a_rotated_identity_unlocks_under_its_new_key_only() {
    let f = Fixture::new();
    let made = create(&f.root, &f.stores(), Some("Turner"), "pass", 1)
        .await
        .unwrap();
    let old = made.public_key.clone();
    drop(made);
    lock(&f.stores()).await;

    std::fs::create_dir_all(f.root.identity_files(&old)).unwrap();
    let next = rekindle_crypto::Identity::generate();
    let secret = *next.secret_key_bytes();
    let pending = journal(secret);
    assert!(matches!(
        rotate(&f.root, &f.stores(), &old, &pending, "wrong", 2).await,
        Err(IdentityError::WrongPassphrase)
    ));
    assert!(!f.root.identity_vault(&next.public_key_hex()).exists());

    let entry = rotate(&f.root, &f.stores(), &old, &pending, "pass", 3)
        .await
        .unwrap();
    assert_eq!(entry.public_key, next.public_key_hex());
    assert_eq!(entry.display_name, "Turner");
    assert!(!f.root.identity_vault(&old).exists());
    assert!(!f.root.identity_db(&old).exists());
    assert!(
        !f.root.identity_files(&old).exists(),
        "the old key's cache names are unreachable under the new key"
    );
    assert!(matches!(
        unlock(&f.root, &f.stores(), &old, "pass", 4).await,
        Err(IdentityError::NotFound)
    ));
    let unlocked = unlock(&f.root, &f.stores(), &entry.public_key, "pass", 5)
        .await
        .unwrap();
    assert_eq!(unlocked.display_name, "Turner");
    assert_eq!(*unlocked.secret, secret);
}

fn journal(secret: [u8; 32]) -> rekindle_vault::typed::pending_rotation::PendingRotation {
    rekindle_vault::typed::pending_rotation::PendingRotation {
        secret: Zeroizing::new(secret),
        mint_seed: Zeroizing::new([4; 32]),
        issued_at_ms: 1,
    }
}

/// A crash after the `IdentityRotated` notices (audit 2026-10-20 R-2):
/// the journal is in the old vault, the registry still names the old key
/// and an interrupted copy may be left. The next unlock of the old key
/// completes the rotation and comes up under the new key, with the
/// journalled set's rows; the journal is gone.
#[tokio::test]
async fn a_rotation_journalled_before_a_crash_completes_at_the_next_unlock() {
    let f = Fixture::new();
    let made = create(&f.root, &f.stores(), Some("Turner"), "pass", 1)
        .await
        .unwrap();
    let old = made.public_key.clone();
    drop(made);
    let next = rekindle_crypto::Identity::generate();
    let pending = journal(*next.secret_key_bytes());
    journal_rotation(&f.stores(), &pending).unwrap();
    // The notices went out here; the process stopped before the switch.
    lock(&f.stores()).await;
    std::fs::write(f.root.identity_db(&next.public_key_hex()), b"torn copy").unwrap();

    let unlocked = unlock(&f.root, &f.stores(), &old, "pass", 2).await.unwrap();
    assert_eq!(unlocked.public_key, next.public_key_hex());
    assert_eq!(*unlocked.secret, *next.secret_key_bytes());
    assert!(!f.root.identity_vault(&old).exists());
    assert!(list(&f.root).unwrap().iter().all(|e| e.public_key != old));
    {
        let guard = f.vault.lock();
        let vault = guard.as_ref().unwrap();
        assert!(rekindle_vault::typed::pending_rotation::load(vault)
            .unwrap()
            .is_none());
    }
    // The rows stored are the set published: the journal re-mints it.
    let mint = || {
        rekindle_crypto::signal::prekey_record::set_values(
            &rekindle_transport::operations::identity::mint_rotation_set(
                &next,
                &pending.mint_seed,
                pending.issued_at_ms,
            ),
        )
        .into_iter()
        .map(|p| p.value)
        .collect::<Vec<_>>()
    };
    assert_eq!(mint(), mint());
}

#[tokio::test]
async fn remove_needs_no_passphrase_and_a_missing_identity_is_not_found() {
    let f = Fixture::new();
    let made = create(&f.root, &f.stores(), None, "pass", 1).await.unwrap();
    let pk = made.public_key.clone();
    drop(made);
    lock(&f.stores()).await;
    std::fs::create_dir_all(f.root.identity_files(&pk)).unwrap();
    remove(&f.root, &pk).unwrap();
    assert!(list(&f.root).unwrap().is_empty());
    assert!(!f.root.identity_vault(&pk).exists());
    assert!(!f.root.identity_files(&pk).exists());
    assert!(matches!(remove(&f.root, &pk), Err(IdentityError::NotFound)));
}
