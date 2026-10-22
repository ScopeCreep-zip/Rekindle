//! The login catch-up of community keys (architecture §7.3; plan E3.1 C2b,
//! moved from the desktop's `mek_rotation::recover_behind_meks`): every
//! community whose key we lack, or hold behind governance's generation
//! (`mek_generation`, restored at login), asks its peers for the current
//! one rather than waiting for traffic to need it. The ask is the shell's
//! retrying recovery on the desktop (`ShellHooks::spawn_community_mek_recovery`,
//! E3.5) and one `RequestMEK` to the top-ranked responder on the daemon,
//! as the daemon's `MekRequest` sends it; the retry and the last-resort
//! mint move with the MEK adapters at E3.5.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance_runtime::GovernanceRuntimeDeps;
use rekindle_types::channel_keys::KeyScope;

use super::HostGovernance;

impl HostGovernance {
    /// The communities whose community key is absent or behind, with our
    /// pseudonym in each.
    pub(crate) fn behind_meks(&self) -> Vec<(String, String)> {
        let keys = self.keys();
        self.ctx
            .communities
            .map()
            .read()
            .values()
            .filter_map(|c| {
                let held = keys
                    .current_epoch(&c.id, KeyScope::Community)
                    .map(|epoch| epoch.0);
                held.is_none_or(|g| g < c.mek_generation)
                    .then(|| Some((c.id.clone(), c.my_pseudonym_key.clone()?)))
                    .flatten()
            })
            .collect()
    }

    /// Ask for every community key that is absent or behind. A no-op when
    /// all are current; run when our route is (re)published, since the
    /// replies need it.
    pub fn recover_behind_meks(&self) {
        for (community_id, pseudonym) in self.behind_meks() {
            if let Some(shell) = &self.ctx.shell {
                shell.spawn_community_mek_recovery(&community_id, &pseudonym);
                continue;
            }
            let request = CommunityEnvelope::Control(ControlPayload::RequestMEK {
                channel_id: KeyScope::Community.wire_channel_str(),
                needed_generation: 0,
                requester_pseudonym: pseudonym,
                cascade_index: 0,
            });
            self.send_to_mesh_impl(&community_id, &request);
        }
    }
}
