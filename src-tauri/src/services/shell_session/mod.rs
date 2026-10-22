//! The GUI's side of the Host's unlock (plan E1.1, ADR 0020): the
//! `ShellHooks` the Host calls at its fixed points, built from the GUI's
//! stages that are not yet ported. The Host owns the lifecycle, the unlock
//! scope, the record pool, its own records, the audit step and the
//! teardown; this runs the rest inside its scope, and each Phase E port
//! deletes its part (`unlocked.rs` names the step that ports each stage).
//! F1 deletes it with the trait.
//!
//! `app` is `None` for the E2E server, which has no window and no node: the
//! loaders run, the network stages do not.

mod ended;
mod governance;
mod unlocked;

use std::sync::Arc;

use rekindle_lifecycle::SessionScope;
use rekindle_node::host::adapters::governance::projection::GovernanceProjected;
use rekindle_node::host::{EndReason, OwnRecordKeys, ShellHooks, UnlockedView};

use crate::state::SharedState;

/// The GUI's `ShellHooks`.
pub struct ShellSession {
    state: SharedState,
    app: Option<tauri::AppHandle>,
}

impl ShellSession {
    #[must_use]
    pub fn new(state: SharedState, app: Option<tauri::AppHandle>) -> Arc<Self> {
        Arc::new(Self { state, app })
    }
}

#[async_trait::async_trait]
impl ShellHooks for ShellSession {
    async fn unlocked(&self, view: &UnlockedView, scope: &Arc<SessionScope>) -> Result<(), String> {
        unlocked::run(&self.state, self.app.as_ref(), view, scope).await
    }

    fn own_records(&self, keys: &OwnRecordKeys) {
        unlocked::own_records(&self.state, keys);
    }

    async fn ending(&self) {
        ended::ending(&self.state, self.app.as_ref()).await;
    }

    async fn ended(&self, reason: EndReason) {
        ended::ended(&self.state, self.app.as_ref(), reason).await;
    }

    fn send_to_mesh(
        &self,
        community_id: &str,
        envelope: &rekindle_codec::community::envelope::CommunityEnvelope,
    ) -> Result<(), String> {
        crate::services::community::send_to_mesh(&self.state, community_id, envelope)
    }

    fn community_left(&self, community_id: &str) {
        crate::state_helpers::forget_community_keys(&self.state, community_id);
    }

    fn spawn_departure_rotation(&self, community_id: &str, departed_pseudonym_hex: &str) {
        let Some(app) = &self.app else {
            return;
        };
        crate::services::community::spawn_departure_rotations(
            app,
            &self.state,
            community_id,
            departed_pseudonym_hex,
        );
    }

    fn require_permission(&self, community_id: &str, perm_bits: u64) -> Result<(), String> {
        crate::commands::community::require_permission(&self.state, community_id, perm_bits)
    }

    fn on_governance_projected(&self, community_id: &str, projected: &GovernanceProjected) {
        governance::on_projected(&self.state, community_id, projected);
    }

    fn spawn_community_mek_recovery(&self, community_id: &str, requester_pseudonym: &str) {
        let Some(app) = &self.app else {
            return;
        };
        crate::services::community::mek_rotation::spawn_community_mek_recovery(
            app.clone(),
            self.state.clone(),
            community_id.to_string(),
            requester_pseudonym.to_string(),
        );
    }

    fn spawn_presence_poll_tick(&self, community_id: &str) {
        governance::spawn_presence_poll_tick(&self.state, community_id);
    }

    fn push_onboarding_complete_to_sync(&self, community_id: &str) {
        governance::push_onboarding_complete_to_sync(&self.state, community_id);
    }

    fn on_mek_installed(
        &self,
        community_id: &str,
        scope: rekindle_types::channel_keys::KeyScope,
        key_bytes: [u8; 32],
        generation: u64,
    ) {
        governance::install_mek(&self.state, community_id, scope, key_bytes, generation);
    }

    fn channel_keys(&self) -> Arc<dyn rekindle_types::channel_keys::ChannelKeyProvider> {
        crate::state_helpers::key_provider(&self.state)
    }

    async fn community_records_ready(
        &self,
        community_id: &str,
        leases: rekindle_records::lease::CommunityLeases,
    ) {
        crate::services::community::leases::records_ready(&self.state, community_id, leases).await;
    }
}
