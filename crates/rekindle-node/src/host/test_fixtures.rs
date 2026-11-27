//! The public Host test fixture (plan GI3): an offline Host over a temp
//! root, an unlocked identity, and the community state the gates judge,
//! built here where `Host.ctx` is visible. This crate's own tests call it,
//! and so do the desktop's relay tests through the `test-fixtures`
//! feature, which only the desktop's `[dev-dependencies]` turn on: no
//! production build compiles this module.

use std::sync::Arc;

use rekindle_db::paths::DataRoot;
use rekindle_governance::state::GovernanceState;
use rekindle_ipc::protocol::{IpcRequest, IpcResponse};
use rekindle_ipc::{Caller, HostApi};
use rekindle_types::id::PseudonymKey;

use super::community::{CommunityState, OwnRemoval};
use super::runtime::{claim, Host, HostConfig, HostStores, TransportSource};
use super::shell_hooks::ShellHooks;

/// A data root laid out under `dir` (nothing created).
#[must_use]
pub fn temp_root(dir: &tempfile::TempDir) -> DataRoot {
    DataRoot::under(dir.path())
}

/// A started Host over `dir` with its own empty stores and no node.
///
/// # Panics
/// If the root cannot be claimed or the Host does not start.
pub async fn offline_host(dir: &tempfile::TempDir) -> Arc<Host> {
    offline_host_with(dir, None).await
}

/// [`offline_host`] with the shell's hooks (`None`: the daemon's mode).
///
/// # Panics
/// If the root cannot be claimed or the Host does not start.
pub async fn offline_host_with(
    dir: &tempfile::TempDir,
    shell: Option<Arc<dyn ShellHooks>>,
) -> Arc<Host> {
    offline_host_on(dir, HostStores::default(), shell).await
}

/// A started Host over `dir` and the caller's `stores` (the desktop passes
/// its `AppState::host_stores()`, so the Host and the shell share one
/// community store and database handle), with no node.
///
/// # Panics
/// If the root cannot be claimed or the Host does not start.
pub async fn offline_host_on(
    dir: &tempfile::TempDir,
    stores: HostStores,
    shell: Option<Arc<dyn ShellHooks>>,
) -> Arc<Host> {
    Host::start(HostConfig {
        root: claim(temp_root(dir)).unwrap(),
        stores,
        transport: TransportSource::Offline,
        serve_bus: false,
        clock: Arc::new(rekindle_utils::SystemClock),
        rng: Arc::new(rekindle_utils::OsRandom),
        idle: Arc::new(rekindle_idle::OsIdle),
        server_query: Arc::new(rekindle_game_detect::server_query::UdpServerQuery),
        shell,
    })
    .await
    .unwrap()
}

/// Create Ann through the window, which unlocks her.
///
/// # Panics
/// If the Host refuses the create.
pub async fn create_ann(host: &Host) -> rekindle_types::dto::identity::IdentityView {
    let answer = host
        .call(
            Caller::shell(),
            IpcRequest::IdentityCreate {
                display_name: Some("Ann".into()),
                passphrase: rekindle_test_fixtures::PASSPHRASE.into(),
            },
        )
        .await;
    match answer {
        IpcResponse::Ok { data, .. } => serde_json::from_value(data).unwrap(),
        other => panic!("create refused: {other:?}"),
    }
}

/// Ask the Host's friend adapter to raise a session-reinitiation review.
///
/// # Panics
/// The host must have an unlocked identity.
pub async fn hold_reinitiation(host: &Host, peer: String) {
    use rekindle_transport::operations::friend::{BackendHold, FriendHost};
    super::adapters::friends::HostFriends::of(&host.ctx)
        .hold_backend_intent(BackendHold::SessionReinitiate { peer })
        .await
        .unwrap();
}

/// Ann's community `c1` in the store with a member registry, and its row.
///
/// # Panics
/// If no identity is unlocked or the row is not written.
pub async fn seed_registry_community(host: &Host, owner: &str) {
    host.ctx.communities.map().write().insert(
        "c1".into(),
        CommunityState {
            member_registry_key: Some("registry".into()),
            ..CommunityState::new("c1")
        },
    );
    let owner = owner.to_string();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            rekindle_db::repo::communities::insert_if_absent(
                conn,
                &owner,
                &rekindle_db::repo::communities::NewCommunity {
                    id: "c1",
                    name: "C",
                    joined_at: 1,
                    my_pseudonym_key: "",
                    mek_generation: 0,
                    member_registry_key: Some("registry"),
                    my_subkey_index: None,
                },
            )
        })
        .await
        .unwrap();
}

/// `community_id`'s merged governance state: `member` holds a creator seat,
/// so every permission the Host's control gates check (plan GI4).
pub fn seat_creator(host: &Host, community_id: &str, member: &PseudonymKey) {
    let mut state = GovernanceState::default();
    state.creators.insert(member.clone());
    host.ctx
        .communities
        .set_governance_state(community_id, state);
}

/// `community_id`'s merged governance state: `author` banned, so departed
/// (the gate counts the left, the removed and the banned alike).
pub fn mark_departed(host: &Host, community_id: &str, author: &PseudonymKey) {
    let mut state = GovernanceState::default();
    state.bans.insert(author.clone());
    host.ctx
        .communities
        .set_governance_state(community_id, state);
}

/// Our own removal from `community_id` noted, as a merged removal notes it.
pub fn note_own_removal(host: &Host, community_id: &str) {
    host.ctx.communities.note_own_removal(
        community_id,
        OwnRemoval {
            banned: false,
            by: None,
            by_name: None,
            reason: None,
        },
    );
}
