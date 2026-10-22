//! The Host's governance projection (plan E3.1 B2c).

use super::*;

/// A rebuild result projects merged channels and roles into the community
/// store and the community's SQLite rows (plan E3.1 B2c), the same body on
/// both hosts.
#[tokio::test]
async fn a_rebuild_result_projects_channels_and_roles_into_the_store_and_sqlite() {
    use rekindle_governance::state::{ChannelState, GovernanceState, RoleState};
    use rekindle_governance_runtime::GovernanceRuntimeDeps;
    use rekindle_types::id::{ChannelId, RoleId};

    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    let db = host.ctx.db.current().unwrap();
    let seeded = owner.clone();
    db.call(move |conn| {
        rekindle_db::repo::communities::insert_if_absent(
            conn,
            &seeded,
            &rekindle_db::repo::communities::NewCommunity {
                id: "c1",
                name: "C",
                my_role_ids: &[],
                joined_at: 1,
                dht_owner_keypair: None,
                my_pseudonym_key: "",
                mek_generation: 0,
                member_registry_key: None,
                my_subkey_index: None,
                governance_key: None,
            },
        )
    })
    .await
    .unwrap();

    let channel = |name: &str, position| ChannelState {
        name: name.into(),
        channel_type: "text".into(),
        record_key: format!("VLD0:{name}"),
        category_id: None,
        position,
        topic: None,
        forum_tags: None,
        slowmode_seconds: None,
        nsfw: None,
        parent_voice_channel_id: None,
        created_lamport: 1,
    };
    let mut gov = GovernanceState::default();
    gov.channels
        .insert(ChannelId([2; 16]), channel("second", 1));
    gov.channels
        .insert(ChannelId([1; 16]), channel("general", 0));
    let admin = RoleId::from_legacy_u32(7);
    gov.roles.insert(
        admin,
        RoleState {
            name: "Admin".into(),
            permissions: 8,
            position: 1,
            color: 0,
            hoist: true,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            lamport: 1,
        },
    );
    host.governance()
        .apply_governance_rebuild_result("c1", gov, 3)
        .await;

    let projected = host
        .ctx
        .communities
        .map()
        .read()
        .get("c1")
        .cloned()
        .unwrap();
    let names: Vec<_> = projected.channels.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["general", "second"]);
    assert_eq!(projected.channel_log_keys.len(), 2);
    assert_eq!(projected.roles.len(), 1);
    assert_eq!(projected.roles[0].name, "Admin");
    assert!(projected.governance_state.is_some());

    let load = db
        .call(move |conn| rekindle_db::repo::communities::load::load(conn, &owner))
        .await
        .unwrap();
    let mut stored: Vec<_> = load.channels.iter().map(|c| c.name.clone()).collect();
    stored.sort();
    assert_eq!(stored, ["general", "second"]);
    let roles: Vec<_> = load
        .roles
        .iter()
        .map(|r| (r.role_id, r.name.clone()))
        .collect();
    assert_eq!(roles, [(RoleId::to_legacy_u32(admin), "Admin".to_string())]);
    host.shutdown().await.unwrap();
}
