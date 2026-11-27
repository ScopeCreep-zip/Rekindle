use super::scan;

#[test]
fn flags_managed_database_access() {
    let bad = r"
            fn a(pool: tauri::State<'_, rekindle_db::Db>) {}
            fn b(app: &tauri::AppHandle) {
                let p = app.try_state::<Db>();
                let q = tauri::Manager::try_state::<crate::db::Db>(app);
                app.manage(pool);
            }
        ";
    assert_eq!(scan("x.rs", bad).len(), 4);
}

#[test]
fn allows_the_handle_and_other_state() {
    let good = r"
            fn a(state: tauri::State<'_, SharedState>, pool: &rekindle_db::Db) {
                let db = state.db.current();
                app.manage(root);
                let k: tauri::State<'_, KeystoreHandle> = app.state();
            }
        ";
    assert!(scan("x.rs", good).is_empty());
}
