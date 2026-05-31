use std::sync::Arc;

use crate::db::DbPool;
use crate::state::AppState;
use tauri::Manager;

pub(crate) fn handle_member_roles_changed(
    app_handle: &tauri::AppHandle,
    state: &Arc<AppState>,
    community_id: &str,
    pseudonym_key: &str,
    role_ids: &[u32],
) {
    use crate::channels::CommunityEvent;

    let my_pseudonym = {
        let communities = state.communities.read();
        communities
            .get(community_id)
            .and_then(|cs| cs.my_pseudonym_key.clone())
    };
    if my_pseudonym.as_deref() == Some(pseudonym_key) && !role_ids.is_empty() {
        let mut communities = state.communities.write();
        if let Some(cs) = communities.get_mut(community_id) {
            cs.my_role_ids = role_ids.to_vec();
        }
    }
    let pool: tauri::State<'_, DbPool> = app_handle.state();
    if let Ok(owner_key) = crate::state_helpers::current_owner_key(state) {
        let cid = community_id.to_string();
        let pk = pseudonym_key.to_string();
        let rids = role_ids.to_vec();
        let is_self = my_pseudonym.as_deref() == Some(pseudonym_key);
        crate::db_helpers::db_fire(&pool, "member_roles_changed_persist", move |conn| {
            let json = serde_json::to_string(&rids).unwrap_or_default();
            conn.execute(
                "UPDATE community_members SET role_ids = ?1 \
                 WHERE owner_key = ?2 AND community_id = ?3 AND pseudonym_key = ?4",
                rusqlite::params![json, owner_key, cid, pk],
            )?;
            if is_self {
                conn.execute(
                    "UPDATE communities SET my_role_ids = ?1 WHERE owner_key = ?2 AND id = ?3",
                    rusqlite::params![json, owner_key, cid],
                )?;
            }
            Ok(())
        });
    }
    crate::event_dispatch::emit_live(
        app_handle,
        "community-event",
        &CommunityEvent::MemberRolesChanged {
            community_id: community_id.to_string(),
            pseudonym_key: pseudonym_key.to_string(),
            role_ids: role_ids.to_vec(),
        },
    );
}
