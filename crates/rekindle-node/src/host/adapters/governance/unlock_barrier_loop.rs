//! The Host's unlock barrier rounds (plan E3.2 S7d-keys.8), on both modes:
//! after the unlock stage rebuilt a community, one task per community per
//! unlock reads our op record's `GovHead` (subkey 2) and asks every known
//! ring peer for its frontier ([`frontier_exchange::request_frontier`]),
//! walking each answer's tips for the highest seq of our device key `d`
//! (`op_walk::walk_seeded`, seeded with the ops we hold). The verdict is
//! [`judge_round`]'s: a round in which at least one known ring peer
//! answered, or an empty ring (a sole creator, on the DHT head alone). A
//! round with no answer is retried on every ring change, every
//! `HeadAdvanced` ([`UnlockBarrier::poke`]) and once per app-call timeout,
//! with no time limit: the plan's rule, "known ring peers that do not
//! answer leave governance writes pending under the visible state". A DHT
//! read that fails leaves the round without a verdict too.

use std::sync::Arc;

use futures::future::join_all;
use rekindle_crypto::group::pseudonym::derive_community_pseudonym;
use rekindle_governance_runtime::op_log::HeldOp;
use rekindle_governance_runtime::unlock_barrier::{
    judge_round, BarrierPhase, BarrierRound, UnlockBarrier,
};
use rekindle_governance_runtime::watch_ring::{ring_successors, RING_K};
use rekindle_governance_runtime::{
    device_signer, frontier_exchange, op_log, op_walk, unlock_barrier, GovernanceRuntimeDeps,
    GovernanceRuntimeError,
};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_types::display_catch_up::CatchUpView;
use rekindle_types::display_catch_up::PendingGovTarget;
use rekindle_types::id::PseudonymKey;

use super::HostGovernance;

/// How far one barrier round got: the known ring peers asked and the
/// answers it had (for the phase log line).
#[derive(Debug, Default)]
struct RoundCounts {
    ring: usize,
    answers: usize,
}

/// `value`, with a debug line naming the round's `exit` when it is `None`
/// (a round with no verdict, or a ring peer that gave no answer).
fn noted<T>(value: Option<T>, community: &str, exit: &'static str) -> Option<T> {
    if value.is_none() {
        tracing::debug!(community, exit, "unlock barrier round: no value");
    }
    value
}

/// The phase as the log line names it.
fn phase_name(phase: BarrierPhase) -> &'static str {
    match phase {
        BarrierPhase::CatchingUp => "catching_up",
        BarrierPhase::Passed => "passed",
        BarrierPhase::Restored(_) => "restored",
    }
}

impl HostGovernance {
    /// Start `community_id`'s barrier rounds once per unlock (the stage
    /// re-runs on a reconnect; the first loop keeps going).
    pub(super) fn spawn_unlock_barrier(&self, community_id: &str) {
        let barrier = self.ctx.communities.unlock_barriers().of(community_id);
        if !barrier.claim_rounds() {
            return;
        }
        let governance = self.clone();
        let community = community_id.to_owned();
        self.ctx
            .community_scope(community_id)
            .spawn_or_drop("unlock barrier", async move {
                governance.barrier_rounds(&community, barrier).await;
            });
    }

    /// Run rounds until the barrier passes or detects a restore. Each phase
    /// change is logged at info with the round's ring size and answers, so
    /// a real-app run shows the barrier (the windows see `CatchUpView`).
    async fn barrier_rounds(&self, community: &str, barrier: Arc<UnlockBarrier>) {
        let mut shown = None;
        loop {
            let mut counts = RoundCounts::default();
            barrier.complete(self.barrier_round(community, &mut counts).await);
            let phase = barrier.phase();
            if shown != Some(phase) {
                shown = Some(phase);
                tracing::info!(
                    community,
                    phase = phase_name(phase),
                    ring = counts.ring,
                    answers = counts.answers,
                    "unlock barrier phase"
                );
            }
            let view = barrier.view().unwrap_or(CatchUpView {
                catching_up: false,
                pending: Vec::new(),
            });
            self.emit_catch_up(community, view);
            match phase {
                BarrierPhase::CatchingUp => {
                    barrier
                        .next_round(rekindle_transport::operations::direct::CALL_DEADLINE)
                        .await;
                }
                BarrierPhase::Passed => {
                    self.update_publication_binding(community, None);
                    return;
                }
                BarrierPhase::Restored(fact) => {
                    tracing::warn!(
                        community,
                        last_seq = fact.last_seq,
                        "unlock barrier: this device's key signed past this copy (a restore)"
                    );
                    self.raise_re_enrol(community).await;
                    return;
                }
            }
        }
    }

    /// One round's verdict; `None` keeps catching up. `counts` gets the
    /// ring's size and the answers as far as the round got.
    async fn barrier_round(
        &self,
        community: &str,
        counts: &mut RoundCounts,
    ) -> Option<BarrierPhase> {
        let state = noted(self.governance_state_impl(community), community, "state")?;
        let device = noted(self.device_op_key(community).ok(), community, "device key")?
            .map(|secret| SigningKey::from_bytes(&secret).verifying_key().to_bytes());
        let store = GovernanceRuntimeDeps::op_store(self);
        let held = noted(
            store.governance_ops(community).await.ok(),
            community,
            "held ops",
        )?;
        // This copy's signed head, never the highest held op of `d` (the
        // sweep stores our own ops that peers echo back).
        let (local_head, dht_head) = if let Some(device) = &device {
            let local_head = noted(
                unlock_barrier::local_head(&*store, community, device)
                    .await
                    .ok(),
                community,
                "local head",
            )?;
            let dht_head = noted(
                self.own_dht_head(community, &state, device, local_head)
                    .await,
                community,
                "dht head",
            )?;
            (local_head, dht_head)
        } else {
            // A device awaiting its first enrolment has no old log.
            (None, None)
        };
        let (me, _) = noted(
            op_log::own_pseudonym(self, community).ok(),
            community,
            "pseudonym",
        )?;
        let ring = ring_successors(community, &state, &me, RING_K);
        counts.ring = ring.len();
        let secret = noted(self.identity_secret_impl(), community, "identity secret")?;
        let signer = derive_community_pseudonym(&secret, community);
        let asks = ring
            .iter()
            .map(|record| self.peer_seq(community, record, &signer, device.as_ref(), &held));
        let answers: Vec<Option<u64>> = join_all(asks).await.into_iter().flatten().collect();
        counts.answers = answers.len();
        judge_round(&BarrierRound {
            local_head,
            dht_head,
            ring_known: !ring.is_empty(),
            answers,
        })
    }

    /// The highest seq of `d` on our op record's log from `local_head` on;
    /// `None` when the read failed, `Some(None)` with no op of `d` there.
    async fn own_dht_head(
        &self,
        community: &str,
        state: &rekindle_governance::state::GovernanceState,
        d: &[u8; 32],
        local_head: Option<u64>,
    ) -> Option<Option<u64>> {
        let record = match device_signer::own_op_record(self, community, state) {
            Ok(record) => record,
            Err(
                GovernanceRuntimeError::DeviceUnendorsed(_)
                | GovernanceRuntimeError::DeviceOpKeyMissing(_),
            ) => return Some(None),
            Err(_) => return None,
        };
        let port = GovernanceRuntimeDeps::record_port(self).ok()?;
        let (read, retirement) = rekindle_records::chain::read_log_with_retirement(
            &*port,
            &record,
            local_head.unwrap_or(0),
        )
        .await
        .ok()?;
        if let Some(pointer) = retirement {
            let received = rekindle_governance_runtime::retirement_pointer::receive(
                self, &*port, community, &record, d, pointer,
            )
            .await
            .ok()?;
            if received {
                rekindle_governance_runtime::rebuild_governance(self, community).await;
            }
        }
        Some(
            (read.entries.iter())
                .filter_map(|e| rekindle_codec::governance_op::decode_op(&e.ciphertext).ok())
                .filter(|op| op.op.device_key == *d)
                .map(|op| op.op.seq)
                .max(),
        )
    }

    /// One ring peer's answer: the highest seq of `d` reachable from its
    /// frontier tips; `None` when it did not answer (or a head failed).
    async fn peer_seq(
        &self,
        community: &str,
        record: &str,
        signer: &SigningKey,
        d: Option<&[u8; 32]>,
        held: &[HeldOp],
    ) -> Option<Option<u64>> {
        let peer = noted(
            self.author_of_record(community, record),
            community,
            "ring author",
        )?;
        let answer = noted(
            frontier_exchange::request_frontier(self, community, signer, |payload| async {
                self.call_member_route(community, &peer, payload)
                    .await
                    .map_err(|error| GovernanceRuntimeError::Adapter(error.to_string()))
            })
            .await
            .ok(),
            community,
            "frontier request",
        )?;
        let Some(d) = d else {
            return Some(None);
        };
        let mut seed = held.to_vec();
        seed.extend(answer.heads);
        let walked = op_walk::walk_seeded(self, community, &answer.frontier.heads, seed)
            .await
            .ok()?;
        Some(
            walked
                .iter()
                .filter(|op| op.device_key == *d)
                .map(|op| op.seq)
                .max(),
        )
    }

    /// Run one barrier round against the simulator's network.
    #[cfg(feature = "netsim-entry")]
    pub async fn netsim_barrier_round(&self, community: &str) -> Option<BarrierPhase> {
        self.barrier_round(community, &mut RoundCounts::default())
            .await
    }

    /// The highest seq this copy signed under `device_key` in
    /// `community_id` (`signed_here` ops, plan E3.2 S7d-keys.3).
    pub(super) async fn signed_here_head_impl(
        &self,
        community_id: &str,
        device_key: &[u8; 32],
    ) -> Result<Option<u64>, GovernanceRuntimeError> {
        let (owner, db) = self.op_store()?;
        let community = community_id.to_owned();
        let device = hex::encode(device_key);
        db.call(move |conn| {
            rekindle_db::repo::governance_ops::signed_here_head(conn, &owner, &community, &device)
        })
        .await
        .map_err(|e| GovernanceRuntimeError::Adapter(format!("signed-here head: {e}")))
    }

    /// What a held write's target shows as (plan E3.2 S7d-keys.8): a
    /// member's card name as the member list publishes it
    /// (`registry.rs`), a channel's or a role's name from the state.
    pub(super) fn pending_target_name_impl(
        &self,
        community: &str,
        target: &PendingGovTarget,
    ) -> Option<String> {
        if let Some(member) = &target.member {
            let key = PseudonymKey::from_hex_lossy(member);
            let card = self.ctx.communities.member_card_name(community, &key);
            return Some(super::community_profile::shown_name(card, &key));
        }
        let state = self.governance_state_impl(community)?;
        if let Some(channel) = &target.channel {
            return (state.channels.iter())
                .find(|(id, _)| id.to_hex() == *channel)
                .map(|(_, c)| c.name.clone());
        }
        let role = target.role.as_ref()?;
        (state.roles.iter())
            .find(|(id, _)| id.to_hex() == *role)
            .map(|(_, r)| r.name.clone())
    }
}
