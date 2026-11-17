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
//!
//! The repair of a missed generation (plan E3.2 S7m item 1c): a member
//! that merges a `MEKGenerationBump` above the key it holds (it was out of
//! the rotator's presence, or unreachable, when the rotator pushed the new
//! key to its online members) asks for that generation with `RequestMEK`
//! ([`HostGovernance::repair_behind_mek`]), a grace after the merge and
//! again on every community sync tick while it stays behind, each tick at
//! the next cascade index, so a responder that cannot reach it yields to
//! the next. Matrix's `m.room_key_request` and Signal's sender-key
//! redistribution make the same choice: the member missing a key asks the
//! holders; the holders answer only a member (`is_key_reader`). The ask is
//! the direct mesh request on both modes, never the desktop recovery's
//! last-resort mint: a bump means a holder exists.

use rekindle_codec::community::envelope::{CommunityEnvelope, ControlPayload};
use rekindle_governance_runtime::GovernanceRuntimeDeps;
use rekindle_types::channel_keys::KeyScope;

use super::HostGovernance;

impl HostGovernance {
    /// The communities whose community key we hold at a generation behind
    /// governance's, with our pseudonym in each. An absent key is not
    /// behind (the joiner waits for the inviter's push); never one we were
    /// removed from (plan E3.2 S9: it asks no peer for a key).
    /// Each comes with the generation governance names.
    pub(crate) fn behind_meks(&self) -> Vec<(String, String, u64)> {
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
                    .then(|| Some((c.id.clone(), c.my_pseudonym_key.clone()?, c.mek_generation)))
                    .flatten()
            })
            .collect()
    }

    /// Ask for every community key held behind its generation. A no-op
    /// when all are current or absent; run when our route is (re)published, since the
    /// replies need it.
    pub fn recover_behind_meks(&self) {
        for (community_id, pseudonym, _) in self.behind_meks() {
            self.request_mek(&community_id, pseudonym);
        }
    }

    /// Ask the mesh for every community key held behind its generation at
    /// `cascade_index` (the sync tick's retry, item 1c). Silent while our
    /// status hides presence: an Invisible member asks no peer for anything.
    pub fn repair_behind_meks(&self, cascade_index: u32) {
        if rekindle_presence::hides_presence(self.ctx.own_presence.status()) {
            return;
        }
        for (community_id, pseudonym, generation) in self.behind_meks() {
            self.send_mek_request(&community_id, pseudonym, generation, cascade_index);
        }
    }

    /// Ask for `community_id`'s key when we hold it behind the generation
    /// governance names (item 1c's trigger, after a merge raised it).
    pub(crate) fn repair_behind_mek(&self, community_id: &str) {
        if rekindle_presence::hides_presence(self.ctx.own_presence.status()) {
            return;
        }
        let behind = self
            .behind_meks()
            .into_iter()
            .find(|(id, _, _)| id == community_id);
        if let Some((community_id, pseudonym, generation)) = behind {
            self.send_mek_request(&community_id, pseudonym, generation, 0);
        }
    }

    /// One `RequestMEK` for `generation` (or newer) to the mesh. It names
    /// only our pseudonym, the scope and a generation every member already
    /// merged; a non-member responder learns nothing new.
    fn send_mek_request(
        &self,
        community_id: &str,
        pseudonym: String,
        generation: u64,
        cascade_index: u32,
    ) {
        tracing::debug!(
            community = community_id,
            generation,
            cascade_index,
            "asking for a community key held behind"
        );
        let request = CommunityEnvelope::Control(ControlPayload::RequestMEK {
            channel_id: KeyScope::Community.wire_channel_str(),
            needed_generation: generation,
            requester_pseudonym: pseudonym,
            cascade_index,
        });
        self.send_to_mesh_impl(community_id, &request);
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
