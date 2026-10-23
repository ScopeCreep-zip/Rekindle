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

use super::ops::pseudonym_of;
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
            self.request_mek(&community_id, pseudonym);
        }
    }

    /// The overlay of `community_id` gained a reachable peer (plan E3.2
    /// S6). While we lack its current key, gossip with no peers dropped
    /// what the peer needs to answer us: our own ops (our `MemberJoined`
    /// above all, which makes us a member in its merged state) and then
    /// the key request. A member that holds the key sends nothing: its
    /// ops are on its `M`/2, and the receiver's op store drops an op it
    /// holds, so a re-sent op is never applied twice.
    pub(crate) fn catch_up_on_peer(&self, community_id: &str) {
        let Some((community, pseudonym)) = self
            .behind_meks()
            .into_iter()
            .find(|(id, _)| id == community_id)
        else {
            return;
        };
        let Some(mine) = pseudonym_of(&pseudonym) else {
            return;
        };
        let host = self.clone();
        self.scope().spawn_or_drop("behind-community catch-up", async move {
            match host.governance_ops(&community).await {
                Ok(held) => {
                    let mut own: Vec<_> = held.into_iter().filter(|op| op.author == mine).collect();
                    own.sort_by_key(|op| op.seq);
                    for op in own {
                        host.send_to_mesh_impl(
                            &community,
                            &CommunityEnvelope::Control(ControlPayload::GovernanceOp {
                                op: op.encoded,
                            }),
                        );
                    }
                }
                Err(error) => {
                    tracing::debug!(community = %community, %error, "catch-up: op store not read");
                }
            }
            host.request_mek(&community, pseudonym);
        });
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
