//! The unlocked identity's preferences (plan D1): stored in its database
//! (`rekindle_db::repo::preferences`) and held in `AppState` while it is
//! unlocked, so synchronous readers (the voice session, auto-away, the
//! relay consent) read them without a database round trip. Every change is
//! written to the database first, then to the copy.

use rekindle_types::preferences::Preferences;

use crate::state::AppState;

/// Load the identity's preferences into the session (unlock).
///
/// # Errors
/// The database could not be read.
pub async fn load(state: &AppState, pool: &rekindle_db::Db, owner: &str) -> Result<(), String> {
    let owner = owner.to_string();
    let prefs = crate::db_helpers::db_call(pool, move |conn| {
        rekindle_db::repo::preferences::load(conn, &owner)
    })
    .await?;
    *state.preferences.write() = prefs;
    Ok(())
}

/// The unlocked identity's preferences (the defaults while locked).
pub fn current(state: &AppState) -> Preferences {
    state.preferences.read().clone()
}

/// Change the preferences: `change` applies to a copy, which is saved,
/// then becomes the session's.
///
/// # Errors
/// No identity is unlocked, or the database refused the values.
pub async fn update(
    state: &std::sync::Arc<AppState>,
    change: impl FnOnce(&mut Preferences),
) -> Result<(), String> {
    let pool = state.db.current()?;
    let owner = crate::state_helpers::current_owner_key(state)?;
    let mut prefs = current(state);
    change(&mut prefs);
    let saved = prefs.clone();
    crate::db_helpers::db_call(&pool, move |conn| {
        rekindle_db::repo::preferences::save(conn, &owner, &saved)
    })
    .await?;
    *state.preferences.write() = prefs;
    Ok(())
}

/// Forget the session's copy (lock).
pub fn reset(state: &AppState) {
    *state.preferences.write() = Preferences::default();
}
