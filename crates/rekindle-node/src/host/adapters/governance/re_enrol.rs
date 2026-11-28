//! The `ReEnrolDevice` review and its commit on the Host (ADR 0022 rules 4
//! and 9, plan E3.2 S7d-keys.10). The review is raised by the Host itself
//! when the unlock barrier finds a restore, when the own-record watch
//! finds our key retired (`OwnKeyRetired`), and at every unlock while the
//! caught-up set still retires our key, so it comes back until committed.
//! The committed action waits unsigned for catch-up, including across
//! unlocks. It writes one `DeviceReEnrol` in every held community.
//! The same commit replaces the prekey record and device id, renews each
//! card's HPKE secret, and queues the contacts' session reset prompts.
//! E5.3 adds the device root key and the DeviceList entry to what it
//! replaces.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::stream::{FuturesUnordered, StreamExt};

use rekindle_governance_runtime::event::GovernanceRuntimeEvent;
use rekindle_governance_runtime::unlock_barrier::{BarrierPhase, UnlockBarrier};
use rekindle_governance_runtime::{
    device_re_enrol, GovernanceRuntimeDeps, GovernanceRuntimeError, OpOutcome,
};
use rekindle_ipc::protocol::{CommunityRequest, IpcRequest, LifecycleRequest};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::display_catch_up::{
    CatchUpView, PendingGovKind, PendingGovOp, PendingGovTarget,
};
use rekindle_types::intent::IntentKind;
use rekindle_vault::{VaultError, VaultKey, VaultStore};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::HostGovernance;

/// The request the review commits.
fn re_enrol_request() -> IpcRequest {
    IpcRequest::Community(CommunityRequest::Lifecycle(
        LifecycleRequest::CommunityDeviceReEnrol {},
    ))
}

/// One `ReEnrolDevice` hold for all communities on this device.
fn same_review(a: &IpcRequest, b: &IpcRequest) -> bool {
    let review = |r: &IpcRequest| {
        matches!(
            r,
            IpcRequest::Community(CommunityRequest::Lifecycle(
                LifecycleRequest::CommunityDeviceReEnrol {}
            ))
        )
    };
    review(a) && review(b)
}

type ReEnrolResults = Vec<(String, Result<OpOutcome, GovernanceRuntimeError>)>;
const RE_ENROL_OP_ID: &str = "device-re-enrol";

/// The review authorizes only the keys held at commit. If a key was
/// already replaced, its signed carrier is recovered by the outbox path.
#[derive(Deserialize, Serialize)]
struct CommittedReEnrol {
    device_id: Option<String>,
    keys: BTreeMap<String, [u8; 32]>,
}

struct HeldCommunity {
    community: String,
    key: [u8; 32],
    barrier: Arc<UnlockBarrier>,
    pending: PendingGovOp,
}

impl HostGovernance {
    /// Raise the backend-originated `ReEnrolDevice` review, found in
    /// `community`.
    pub(super) async fn raise_re_enrol(&self, community: &str) {
        if !matches!(self.re_enrol_is_held(), Ok(false)) {
            return;
        }
        let raised = crate::host::backend_hold::raise(
            &self.ctx,
            re_enrol_request(),
            None,
            IntentKind::ReEnrolDevice,
            same_review,
        )
        .await;
        match raised {
            Ok(()) => tracing::info!(community, "device re-enrolment review raised"),
            Err(refused) => {
                tracing::warn!(community, ?refused, "device re-enrolment review not raised");
            }
        }
    }

    /// [`Self::raise_re_enrol`] on `community`'s scope (from a sync path).
    pub(super) fn spawn_re_enrol_review(&self, community: &str) {
        let governance = self.clone();
        let found = community.to_owned();
        self.ctx.community_scope(community).spawn_or_drop(
            "device re-enrolment review",
            async move {
                governance.raise_re_enrol(&found).await;
            },
        );
    }

    /// Whether this device must re-enrol in `community`.
    pub(super) fn needs_re_enrol(&self, community: &str) -> bool {
        match device_re_enrol::needed(self, community) {
            Ok(needed) => needed,
            Err(error) => {
                tracing::debug!(community, %error, "re-enrolment check skipped");
                false
            }
        }
    }

    /// The communities we hold and were not removed from, in id order.
    fn sorted_held_communities(&self) -> Vec<String> {
        let mut held = self.held_communities();
        held.sort();
        held
    }

    /// The review's `lastSeq`: the lowest over the communities that found
    /// a restore or a retirement of our key; `None` when none did.
    pub(crate) async fn re_enrol_last_seq(&self) -> Option<u64> {
        let mut lowest = None;
        for community in self.sorted_held_communities() {
            if !self.needs_re_enrol(&community) {
                continue;
            }
            match device_re_enrol::last_seq(self, &community).await {
                Ok(seq) => lowest = Some(lowest.map_or(seq, |l: u64| l.min(seq))),
                Err(error) => tracing::debug!(community, %error, "re-enrolment seq not read"),
            }
        }
        lowest
    }

    /// The commit: one `DeviceReEnrol` in every community this data root
    /// holds (a copied data root copied every community's key, ADR 0022
    /// rule 4), each answered.
    pub(crate) async fn re_enrol_everywhere(&self) -> Result<ReEnrolResults, String> {
        if self.re_enrol_is_held()? {
            return Err("device re-enrolment is already waiting for catch-up".into());
        }
        let _re_enrol = self.ctx.device_re_enrol.lock().await;
        if self.re_enrol_is_held()? {
            return Err("device re-enrolment is already waiting for catch-up".into());
        }
        let communities = self.sorted_held_communities();
        if !communities
            .iter()
            .any(|community| self.needs_re_enrol(community))
        {
            return Ok(Vec::new());
        }
        if communities
            .iter()
            .any(|community| self.unlock_barrier(community).phase() == BarrierPhase::CatchingUp)
        {
            let mut keys = BTreeMap::new();
            let mut unavailable = Vec::new();
            for community in communities {
                if let Some(key) = self.re_enrol_key(&community)? {
                    keys.insert(community, key);
                } else {
                    let error = GovernanceRuntimeError::DeviceOpKeyMissing(community.clone());
                    unavailable.push((community, Err(error)));
                }
            }
            if keys.is_empty() {
                return Ok(unavailable);
            }
            let committed = CommittedReEnrol {
                device_id: self.re_enrol_device_id().await?,
                keys,
            };
            let bytes = serde_json::to_vec(&committed).map_err(|error| error.to_string())?;
            self.with_re_enrol_vault(|vault| vault.put(&VaultKey::PendingDeviceReEnrol, &bytes))?;
            let mut results = self.hold_re_enrol(committed)?;
            results.append(&mut unavailable);
            results.sort_by(|(a, _), (b, _)| a.cmp(b));
            return Ok(results);
        }
        crate::host::own_prekeys::replace_for_re_enrol(&self.ctx).await?;
        self.write_re_enrol(communities).await
    }

    fn with_re_enrol_vault<T>(
        &self,
        use_vault: impl FnOnce(&VaultStore) -> Result<T, VaultError>,
    ) -> Result<T, String> {
        let guard = self.ctx.device_vault.lock();
        let vault = guard.as_ref().ok_or("the device vault is locked")?;
        use_vault(vault).map_err(|error| error.to_string())
    }

    fn re_enrol_key(&self, community: &str) -> Result<Option<[u8; 32]>, String> {
        self.device_op_key(community)
            .map(|key| key.map(|secret| SigningKey::from_bytes(&secret).verifying_key().to_bytes()))
            .map_err(|error| error.to_string())
    }

    async fn re_enrol_device_id(&self) -> Result<Option<String>, String> {
        let (owner, db) = self.owner_and_db().ok_or("the identity is locked")?;
        db.call(move |conn| rekindle_db::repo::identity::login_row(conn, &owner))
            .await
            .map_err(|error| error.to_string())?
            .map(|(_, row)| row.device_id)
            .ok_or_else(|| "no identity row is held".into())
    }

    /// Resume the approval before the network stage can raise another
    /// review for the same key.
    pub(crate) fn resume_held_re_enrol(&self) -> Result<(), String> {
        if let Some(bytes) =
            self.with_re_enrol_vault(|vault| vault.get(&VaultKey::PendingDeviceReEnrol))?
        {
            let committed = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
            self.hold_re_enrol(committed)?;
        }
        Ok(())
    }

    fn re_enrol_is_held(&self) -> Result<bool, String> {
        self.with_re_enrol_vault(|vault| vault.get(&VaultKey::PendingDeviceReEnrol))
            .map(|pending| pending.is_some())
    }

    fn hold_re_enrol(&self, committed: CommittedReEnrol) -> Result<ReEnrolResults, String> {
        let scope = self
            .ctx
            .unlock_scope
            .read()
            .clone()
            .ok_or("the identity is locked")?;
        let mut held = Vec::new();
        let mut results = Vec::new();
        for (community, key) in &committed.keys {
            if !self.holds_community(community)
                || self.re_enrol_key(community)?.as_ref() != Some(key)
            {
                continue;
            }
            let barrier = self.unlock_barrier(community);
            let pending = PendingGovOp::held_as(
                RE_ENROL_OP_ID.to_owned(),
                PendingGovKind::DeviceReEnrol,
                PendingGovTarget::default(),
            );
            barrier.hold(pending.clone());
            self.announce_re_enrol(community);
            results.push((
                community.clone(),
                Ok(OpOutcome::Held {
                    op_id: pending.op_id.clone(),
                }),
            ));
            held.push(HeldCommunity {
                community: community.clone(),
                key: *key,
                barrier,
                pending,
            });
        }
        let host = self.clone();
        scope
            .spawn_with_token("held device re-enrolment", move |stop| async move {
                Box::pin(host.finish_held_re_enrol(held, committed.device_id, stop)).await;
            })
            .map_err(|error| error.to_string())?;
        Ok(results)
    }

    async fn finish_held_re_enrol(
        &self,
        held: Vec<HeldCommunity>,
        device_id: Option<String>,
        stop: CancellationToken,
    ) {
        let mut ready: FuturesUnordered<_> = held
            .into_iter()
            .map(|held| {
                let stop = self.ctx.community_scope(&held.community).token();
                async move {
                    let ready = stop
                        .run_until_cancelled(held.barrier.wait_for_verdict())
                        .await
                        .is_some();
                    (held, ready)
                }
            })
            .collect();

        let mut prekeys_ready = false;
        while let Some(next) = stop.run_until_cancelled(ready.next()).await {
            let Some((held, reached_verdict)) = next else {
                break;
            };
            let _re_enrol = self.ctx.device_re_enrol.lock().await;
            let community = &held.community;
            if reached_verdict && self.holds_community(community) {
                held.barrier.signing(RE_ENROL_OP_ID);
                self.announce_re_enrol(community);
                let result = async {
                    if self.re_enrol_key(community)? != Some(held.key) {
                        return Ok(());
                    }
                    if !prekeys_ready && self.re_enrol_device_id().await? == device_id {
                        crate::host::own_prekeys::replace_for_re_enrol(&self.ctx).await?;
                    }
                    prekeys_ready = true;
                    self.write_re_enrol_in(community)
                        .await
                        .map(drop)
                        .map_err(|error| error.to_string())
                }
                .await;
                if let Err(reason) = result {
                    self.emit_event(GovernanceRuntimeEvent::HeldWriteRefused {
                        community_id: community.clone(),
                        label: Some(held.pending.label),
                        reason,
                    });
                }
            }
            held.barrier.release(RE_ENROL_OP_ID);
            self.announce_re_enrol(community);
        }
        if stop.is_cancelled() {
            return;
        }
        let _re_enrol = self.ctx.device_re_enrol.lock().await;
        // If every key was replaced before an interruption, only the
        // post-cutover prompts and approval cleanup remain.
        if prekeys_ready
            || self
                .re_enrol_device_id()
                .await
                .is_ok_and(|current| current != device_id)
        {
            if let Err(error) = self.prompt_session_resets().await {
                tracing::warn!(%error, "re-enrolment session prompts not queued");
            }
        }
        if let Err(error) =
            self.with_re_enrol_vault(|vault| vault.delete(&VaultKey::PendingDeviceReEnrol))
        {
            tracing::warn!(%error, "committed re-enrolment could not be cleared");
        }
    }

    fn announce_re_enrol(&self, community: &str) {
        self.emit_catch_up(
            community,
            self.unlock_barrier(community)
                .view()
                .unwrap_or(CatchUpView {
                    catching_up: false,
                    pending: Vec::new(),
                }),
        );
    }

    async fn write_re_enrol(&self, communities: Vec<String>) -> Result<ReEnrolResults, String> {
        let mut written = Vec::new();
        for community in communities {
            let outcome = self.write_re_enrol_in(&community).await;
            written.push((community, outcome));
        }
        self.prompt_session_resets().await?;
        if written.iter().all(|(_, result)| result.is_ok()) {
            self.ctx.intents.retain_held(|owner, action| {
                owner.conn.is_some() || !same_review(action, &re_enrol_request())
            });
        }
        Ok(written)
    }

    async fn write_re_enrol_in(
        &self,
        community: &str,
    ) -> Result<OpOutcome, GovernanceRuntimeError> {
        let previous_record = self
            .own_op_record_writer(community)
            .ok()
            .map(|(record, _)| record);
        let outcome = Box::pin(device_re_enrol::re_enrol(self, community)).await?;
        if let Some(record) = previous_record {
            self.stop_member_record_republishes(&record);
        }
        self.persist_governance_snapshot(community).await;
        if let Err(error) = self.publish_re_enrolled_card(community).await {
            tracing::warn!(community, %error, "re-enrolment could not update the member card");
        }
        self.wake_device_publication(community);
        Ok(outcome)
    }

    async fn prompt_session_resets(&self) -> Result<(), String> {
        use rekindle_types::friend_store::FriendshipState;
        let owner = self
            .ctx
            .require_owner()
            .map_err(|_| "the identity is locked")?;
        let db = self.ctx.db.current().map_err(|e| e.to_string())?;
        let contacts = db
            .call(move |conn| rekindle_db::repo::friends::load_all(conn, &owner))
            .await
            .map_err(|e| e.to_string())?;
        let host = crate::host::adapters::friends::HostFriends::of(&self.ctx);
        for contact in contacts
            .into_iter()
            .filter(|c| c.friendship_state == FriendshipState::Accepted)
        {
            if let Err(error) = rekindle_transport::operations::friend::session_reset::start(
                &host,
                &contact.public_key,
            )
            .await
            {
                tracing::warn!(%error, peer = %contact.public_key, "re-enrolment session prompt not queued");
            }
        }
        Ok(())
    }
}
