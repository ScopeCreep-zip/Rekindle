//! A join completed during a shell session hands the community's records
//! to the shell (Linux finding IB1): the desktop starts the community's
//! presence poll, inspect loop and keepalive only from
//! `ShellHooks::community_records_ready`, and the Host's join never called
//! it, so a shell-mode joiner wrote no presence row until its next unlock
//! and its peers read it as offline.

use std::sync::Arc;

use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_node::host::adapters::governance::projection::GovernanceProjected;
use rekindle_node::host::{EndReason, OwnRecordKeys, ShellHooks, UnlockedView};

use crate::harness::{start_with, Community, COMMUNITY_NAME};
use crate::shell_node::SimMode;

/// The communities whose records the Host handed to the shell, in order;
/// every other hook answers as a shell that holds nothing.
#[derive(Default)]
struct Shell(parking_lot::Mutex<Vec<String>>);

#[async_trait::async_trait]
impl ShellHooks for Shell {
    async fn unlocked(
        &self,
        _view: &UnlockedView,
        _scope: &Arc<rekindle_lifecycle::SessionScope>,
    ) -> Result<(), String> {
        Ok(())
    }
    fn own_records(&self, _keys: &OwnRecordKeys) {}
    async fn ending(&self) {}
    async fn ended(&self, _reason: EndReason) {}
    fn own_gossip_signed(&self, _community_id: &str, _sender: &str, _dedup_key: &str) {}
    fn community_left(&self, _community_id: &str) {}
    fn spawn_departure_rotation(&self, _community_id: &str, _departed: &str) {}
    fn on_governance_projected(&self, _community_id: &str, _projected: &GovernanceProjected) {}
    fn spawn_community_mek_recovery(&self, _community_id: &str, _requester_pseudonym: &str) {}
    fn spawn_presence_poll_tick(&self, _community_id: &str) {}
    async fn presence_refresh(&self, _community_id: &str) {}
    fn push_onboarding_complete_to_sync(&self, _community_id: &str) {}
    fn on_mek_installed(
        &self,
        _community_id: &str,
        _scope: rekindle_types::channel_keys::KeyScope,
        _key_bytes: [u8; 32],
        _generation: u64,
    ) {
    }
    fn channel_keys(&self) -> Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider> {
        rekindle_node::daemon::mek_rotation::provider_over(
            Arc::new(parking_lot::RwLock::new(
                rekindle_transport::crypto::mek::MekCache::new(),
            )),
            rekindle_vault::new_handle(),
        )
    }
    fn session_presence(&self) -> Arc<dyn rekindle_presence::SessionPresence> {
        Arc::new(crate::sim_session_presence::SimSessionPresence::default())
    }
    async fn community_records_ready(
        &self,
        community_id: &str,
        _leases: rekindle_records::lease::CommunityLeases,
    ) {
        self.0.lock().push(community_id.to_owned());
    }
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn a_shell_join_hands_the_records_to_the_shell() {
    let shell = Arc::new(Shell::default());
    let joiner_shell = Arc::clone(&shell) as Arc<dyn ShellHooks>;
    let (sim, mut nodes) = start_with(2, 977, &|i| {
        (
            SimMode::Daemon,
            (i == 1).then(|| Arc::clone(&joiner_shell) as _),
        )
    })
    .await;
    for (i, node) in nodes.iter_mut().enumerate() {
        node.unlock_new_identity(&format!("member{i}")).await;
    }
    let created = nodes[0]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityCreate {
                name: COMMUNITY_NAME.into(),
                description: String::new(),
                approval_required: false,
                co_creators: Vec::new(),
            },
        )))
        .await;
    let anchor = created["anchor"].as_str().expect("an anchor").to_owned();
    let mut community = Community {
        sim,
        nodes,
        anchor: anchor.clone(),
        link: String::new(),
    };
    let link = community.invite().await;
    assert!(shell.0.lock().is_empty(), "{:?}", shell.0.lock());
    community.nodes[1]
        .ok(IpcRequest::Community(CommunityRequest::Lifecycle(
            LifecycleRequest::CommunityJoin { invite: link },
        )))
        .await;
    // Handed over by the join itself, before it answers: no unlock or
    // reconnect heal is needed for the shell to start the loops.
    assert_eq!(
        *shell.0.lock(),
        [anchor],
        "the shell holds the joined records"
    );
}
