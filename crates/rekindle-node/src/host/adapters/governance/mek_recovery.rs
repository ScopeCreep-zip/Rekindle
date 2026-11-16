//! The login catch-up of community keys (architecture §7.3; plan E3.1 C2b,
//! moved from the desktop's `mek_rotation::recover_behind_meks`): every
//! community whose key we hold behind governance's generation
//! (`mek_generation`, restored at login) asks its peers for the current
//! one rather than waiting for traffic to need it. A community whose key
//! we never held is not asked for: a joiner's first key is the inviter's
//! `MekTransfer` push (plan E3.2 S7 item 7, D16), and `RequestMEK` stays
//! only for a member behind on a generation (register S7l, HIGH-1). The ask is the shell's
//! retrying recovery on the desktop (`ShellHooks::spawn_community_mek_recovery`,
//! E3.5) and one `RequestMEK` to the top-ranked responder on the daemon,
//! as the daemon's `MekRequest` sends it; the retry and the last-resort
//! mint move with the MEK adapters at E3.5.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance_runtime::GovernanceRuntimeDeps;
use rekindle_types::channel_keys::KeyScope;

use super::HostGovernance;

impl HostGovernance {
    /// The communities whose community key we hold at a generation behind
    /// governance's, with our pseudonym in each. An absent key is not
    /// behind (the joiner waits for the inviter's push); never one we were
    /// removed from (plan E3.2 S9: it asks no peer for a key).
    pub(crate) fn behind_meks(&self) -> Vec<(String, String)> {
        let keys = self.keys();
        self.ctx
            .communities
            .map()
            .read()
            .values()
            .filter(|c| c.own_removal.is_none())
            .filter_map(|c| {
                let held = keys
                    .current_epoch(&c.id, KeyScope::Community)
                    .map(|epoch| epoch.0);
                held.is_some_and(|g| g < c.mek_generation)
                    .then(|| Some((c.id.clone(), c.my_pseudonym_key.clone()?)))
                    .flatten()
            })
            .collect()
    }

    /// Ask for every community key held behind its generation. A no-op
    /// when all are current or absent; run when our route is (re)published, since the
    /// replies need it.
    pub fn recover_behind_meks(&self) {
        for (community_id, pseudonym) in self.behind_meks() {
            self.request_mek(&community_id, pseudonym);
        }
    }

    fn request_mek(&self, community_id: &str, pseudonym: String) {
        if let Some(shell) = &self.ctx.shell {
            shell.spawn_community_mek_recovery(community_id, &pseudonym);
            return;
        }
        let request = CommunityEnvelope::Control(ControlPayload::RequestMEK {
            channel_id: KeyScope::Community.wire_channel_str(),
            needed_generation: 0,
            requester_pseudonym: pseudonym,
            cascade_index: 0,
        });
        self.send_to_mesh_impl(community_id, &request);
    }
}
