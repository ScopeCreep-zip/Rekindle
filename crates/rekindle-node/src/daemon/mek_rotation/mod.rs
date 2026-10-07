//! Departure-triggered MEK rotation on the daemon track.
//!
//! `communities-channels.md` makes this the forward-secrecy mechanism:
//! when a member departs, every remaining member independently computes
//! `blake3(departed || own_pseudonym)`, the lowest hash rotates, and the
//! new key is delivered to each peer by `app_call`. The shared
//! implementation is `rekindle-mek-rotation`; this module is the
//! daemon's adapter for it.
//!
//! Before this, the daemon had none. `spawn_text_mek_rotation_for_ban`
//! traced a line and returned, and the leave RPC rekeyed by writing a
//! v1.0 registry MEK vault that only an "operator" could write. A
//! banned member on the daemon track kept a working key.
//!
//! ## Why a worker channel rather than a spawned task
//!
//! Rotation is fire-and-forget from the caller's perspective — a ban has
//! already landed and must not be undone by a rotation failure — but it
//! is *slow*: `wait_for_rotation_slot` sleeps through the cascade levels
//! so a lower-ranked member only rotates if the elected one stays quiet.
//! Running that inline would hold an IPC handler open for tens of
//! seconds.
//!
//! Detaching it needs `'static` state, and the daemon's single
//! `Arc<DaemonContext>` deliberately stops at `dispatch`: 68 handler
//! functions take `&DaemonContext`, and threading an `Arc` through all
//! of them to reach two call sites would be a poor trade. So the
//! triggers send a request on a channel, and one worker — spawned at
//! startup where the `Arc` does exist — owns it and does the waiting.
//! Both current triggers (moderation ban, leave notification) and any
//! future one only need a `Sender`.

mod cache;
mod deps_impl;

use std::sync::Arc;

use crate::daemon::dispatch::DaemonContext;

pub use cache::{key_provider, DaemonMekPersist, MekCacheAdapter};

/// Why a MEK needs replacing.
#[derive(Debug, Clone)]
pub enum MekRotationKind {
    /// A member left or was banned and still holds the current key.
    /// Which peer rotates is decided by `blake3(departed || self)`, so
    /// every recipient of the same departure queues this and exactly one
    /// of them does the work.
    Departure { departed_pseudonym_hex: String },
    /// A gossip `Kick` (plan D20: kick rotates). The kicked member still
    /// holds the current key. Acted on only when `kicker` holds
    /// KICK_MEMBERS under the merged governance — gossip is unauthenticated
    /// as to authority, and an unchecked kick would let anyone force
    /// rotations. Then it is a departure of `target`.
    Kick {
        target_pseudonym_hex: String,
        kicker_pseudonym_hex: String,
    },
    /// An operator asked for a specific key (community or channel) to be
    /// replaced.
    ///
    /// No election: there is no departure to seed one with, and the
    /// request names *this* node as the initiator, which must hold a
    /// rotation permission (`rekindle_governance::validate`, plan D20).
    Manual {
        scope: rekindle_types::channel_keys::KeyScope,
    },
}

/// A reason to replace a community's MEK, queued for the worker.
#[derive(Debug, Clone)]
pub struct MekRotationRequest {
    /// Governance key of the affected community.
    pub community_id: String,
    pub kind: MekRotationKind,
}

impl MekRotationRequest {
    /// A departure-triggered rotation.
    #[must_use]
    pub fn departure(
        community_id: impl Into<String>,
        departed_pseudonym_hex: impl Into<String>,
    ) -> Self {
        Self {
            community_id: community_id.into(),
            kind: MekRotationKind::Departure {
                departed_pseudonym_hex: departed_pseudonym_hex.into(),
            },
        }
    }

    /// A rotation for a gossip `Kick` of `target` sent by `kicker`.
    #[must_use]
    pub fn kick(
        community_id: impl Into<String>,
        target_pseudonym_hex: impl Into<String>,
        kicker_pseudonym_hex: impl Into<String>,
    ) -> Self {
        Self {
            community_id: community_id.into(),
            kind: MekRotationKind::Kick {
                target_pseudonym_hex: target_pseudonym_hex.into(),
                kicker_pseudonym_hex: kicker_pseudonym_hex.into(),
            },
        }
    }

    /// An operator-requested rotation of one key.
    #[must_use]
    pub fn manual(
        community_id: impl Into<String>,
        scope: rekindle_types::channel_keys::KeyScope,
    ) -> Self {
        Self {
            community_id: community_id.into(),
            kind: MekRotationKind::Manual { scope },
        }
    }
}

/// Handle used by triggers to ask for a rotation.
pub type MekRotationSender = tokio::sync::mpsc::UnboundedSender<MekRotationRequest>;

/// Receiving half, owned by the worker.
pub type MekRotationReceiver = tokio::sync::mpsc::UnboundedReceiver<MekRotationRequest>;

/// Create the trigger/worker channel pair.
#[must_use]
pub fn channel() -> (MekRotationSender, MekRotationReceiver) {
    tokio::sync::mpsc::unbounded_channel()
}

/// The daemon's `MekDistributeDeps` implementation.
///
/// Owns an `Arc<DaemonContext>` rather than borrowing, because it
/// outlives the request that triggered it — see the module docs.
pub struct DaemonMekAdapter {
    pub(super) ctx: Arc<DaemonContext>,
    pub(super) cache: Arc<dyn rekindle_mek_rotation::ChannelMekCache>,
    pub(super) persist: Arc<dyn rekindle_mek_rotation::MekPersist>,
}

impl DaemonMekAdapter {
    #[must_use]
    pub fn new(ctx: Arc<DaemonContext>) -> Self {
        let cache: Arc<dyn rekindle_mek_rotation::ChannelMekCache> =
            Arc::new(MekCacheAdapter::new(Arc::clone(&ctx.mek_cache)));
        let persist: Arc<dyn rekindle_mek_rotation::MekPersist> = Arc::new(DaemonMekPersist);
        Self {
            ctx,
            cache,
            persist,
        }
    }
}

/// Drain rotation requests, one at a time, until the daemon shuts down.
///
/// Serialised deliberately. Two concurrent rotations of the same
/// community would race to bump the generation and each would see the
/// other's `MEKGenerationBump` as a competing write; running them in
/// order means the second observes the first's generation and yields
/// through `wait_for_rotation_slot` instead of duplicating the work.
pub async fn run_worker(ctx: Arc<DaemonContext>, mut rx: MekRotationReceiver) {
    tracing::info!("MEK rotation worker started");
    while let Some(Some(request)) = ctx.shutdown.run_until(rx.recv()).await {
        // A rotation belongs to the unlock that queued it: dropped while
        // locked, and cut off when a lock starts (it needs the signing key).
        let unlock = ctx.unlock_scope_or_closed();
        if unlock.is_closed() {
            tracing::debug!(community = %request.community_id, "locked — rotation request dropped");
            continue;
        }
        let adapter = DaemonMekAdapter::new(Arc::clone(&ctx));
        let community = &request.community_id;
        let rotation = async {
            match &request.kind {
                MekRotationKind::Departure {
                    departed_pseudonym_hex,
                } => {
                    rekindle_mek_rotation::rotate_text_mek_for_departure(
                        &adapter,
                        &request.community_id,
                        departed_pseudonym_hex,
                    )
                    .await
                }
                MekRotationKind::Kick {
                    target_pseudonym_hex,
                    kicker_pseudonym_hex,
                } => {
                    let kicker =
                        rekindle_types::id::PseudonymKey::from_hex_lossy(kicker_pseudonym_hex);
                    let authorized = ctx
                        .community_runtime
                        .governance_state(&request.community_id)
                        .is_some_and(|state| {
                            rekindle_types::permissions::Permissions(
                                rekindle_governance::permissions::compute_permissions(
                                    &kicker, None, &state, 0,
                                ),
                            )
                            .has(rekindle_types::permissions::KICK_MEMBERS)
                        });
                    if authorized {
                        rekindle_mek_rotation::rotate_text_mek_for_departure(
                            &adapter,
                            &request.community_id,
                            target_pseudonym_hex,
                        )
                        .await
                    } else {
                        tracing::debug!(community = %community, "kick from a member without KICK_MEMBERS — no rotation");
                        Ok(())
                    }
                }
                MekRotationKind::Manual { scope } => {
                    rekindle_mek_rotation::rotate_mek_on_request(
                        &adapter,
                        &request.community_id,
                        *scope,
                    )
                    .await
                }
            }
        };
        // A lock or exit closes the unlock scope, and the rotation stops at
        // its next safe point (the cascade wait or the next recipient): an
        // unrotated key is the same outcome as a rotation nobody was online
        // to receive, which the protocol repairs. No Veilid call is cut.
        let outcome = rotation.await;
        if unlock.is_closed() {
            tracing::debug!(community = %community, "rotation stopped by lock");
        }
        match outcome {
            Ok(()) => tracing::debug!(community = %community, "rotation finished"),
            // Not an error path in the common case: losing the cascade
            // to a better-ranked peer, or having no online recipients,
            // both end here and both are correct outcomes.
            Err(e) => tracing::debug!(
                community = %community,
                error = %e,
                "rotation did not complete"
            ),
        }
    }
    tracing::info!("MEK rotation worker stopped");
}
