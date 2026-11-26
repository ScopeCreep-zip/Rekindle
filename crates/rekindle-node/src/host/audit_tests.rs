use super::*;
use crate::host::fixture::Fixture;

#[tokio::test]
async fn appended_entries_verify_and_survive_a_relock() {
    let f = Fixture::new();
    let made = crate::host::identity::create(
        &f.root,
        &f.stores(),
        None,
        "pass",
        1,
        &rekindle_utils::random::OsRandom,
    )
    .await
    .unwrap();
    let owner = made.public_key.clone();
    drop(made);
    let db = f.db.current().unwrap();
    let slot = ChainSlot::new(None);
    assert_eq!(
        restore(&slot, &db, &f.vault, &owner).await.unwrap(),
        Restored::Intact
    );
    append(
        &slot,
        &db,
        &f.vault,
        &owner,
        AuditKind::VaultUnlocked,
        serde_json::json!({}),
        1,
    );
    record_unlock_failures(&slot, &db, &f.vault, &owner, 3, 2);
    record_unlock_failures(&slot, &db, &f.vault, &owner, 0, 3);
    assert_eq!(
        verify(&slot, &db, &owner).await,
        Verified::Intact { length: 2 }
    );

    close(&slot);
    assert_eq!(
        restore(&slot, &db, &f.vault, &owner).await.unwrap(),
        Restored::Intact
    );
    append(
        &slot,
        &db,
        &f.vault,
        &owner,
        AuditKind::Request,
        serde_json::json!({}),
        4,
    );
    assert_eq!(
        verify(&slot, &db, &owner).await,
        Verified::Intact { length: 3 }
    );
    assert_eq!(export(&db, &owner, 1).await.unwrap().len(), 2);
}
