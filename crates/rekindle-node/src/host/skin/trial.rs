//! "Keep this skin?" (plan E7.7.2 "Apply"; Telegram's theme preview):
//! choosing a library skin shows it in every window for
//! [`SKIN_TRIAL_MS`] without storing it. The Host holds the trial and
//! puts the stored choice back at its end unless `SkinKeep` keeps it, so a
//! skin that hides the controls cannot strand the user, and a crash or a
//! lock mid-trial leaves the stored choice untouched. The windows only
//! render the prompt from `SystemEvent::SkinTrialChanged` and send
//! `SkinKeep`; the CLI and TUI settle it the same way.

use std::time::Duration;

use rekindle_db::repo::{preferences, skins};
use rekindle_types::dto::skin::SkinManifest;
use rekindle_types::dto::skin_library::{SkinTrial, SKIN_TRIAL_MS};
use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

use super::{failed, not_found, serve, Answer};
use crate::daemon::dispatch::DaemonContext;

/// The trial in progress, for one identity.
struct Held {
    owner: String,
    trial: SkinTrial,
    /// Which trial a revert timer belongs to: a newer trial (or a settled
    /// one) makes an older timer's revert a no-op.
    seq: u64,
}

#[derive(Default)]
struct Inner {
    held: Option<Held>,
    next_seq: u64,
}

/// The Host's one skin trial slot (`DaemonContext::skin_trial`).
#[derive(Default)]
pub(crate) struct TrialSlot {
    inner: parking_lot::Mutex<Inner>,
}

impl TrialSlot {
    /// The trial `owner` has in progress.
    pub(crate) fn current(&self, owner: &str) -> Option<SkinTrial> {
        self.inner
            .lock()
            .held
            .as_ref()
            .filter(|h| h.owner == owner)
            .map(|h| h.trial.clone())
    }

    /// End any trial without restoring anything: the identity is locking,
    /// and its stored choice never changed.
    pub(crate) fn clear(&self) {
        self.inner.lock().held = None;
    }

    fn begin(&self, owner: String, trial: SkinTrial) -> u64 {
        let mut inner = self.inner.lock();
        inner.next_seq += 1;
        let seq = inner.next_seq;
        inner.held = Some(Held { owner, trial, seq });
        seq
    }

    /// Take the trial if `pick` matches it.
    fn take_if(&self, pick: impl FnOnce(&Held) -> bool) -> Option<SkinTrial> {
        let mut inner = self.inner.lock();
        if inner.held.as_ref().is_some_and(pick) {
            inner.held.take().map(|h| h.trial)
        } else {
            None
        }
    }
}

fn publish(ctx: &DaemonContext, trial: Option<SkinTrial>) {
    ctx.publish_event(SubscriptionEvent::System(SystemEvent::SkinTrialChanged {
        trial,
    }));
}

/// Put `pkg:<hash>` on trial: every window shows it, the stored choice
/// stays, and [`SKIN_TRIAL_MS`] later the Host reverts it unless kept.
/// A trial replacing another keeps the first one's `previous`.
pub(super) async fn begin(ctx: &DaemonContext, id: String, hash: String) -> Answer<()> {
    let (db, owner) = (ctx.require_db()?, ctx.require_owner()?);
    let who = owner.clone();
    let (manifest, stored) = db
        .call(move |conn| {
            let manifest = skins::list(conn, &who)?
                .into_iter()
                .find(|r| r.hash == hash)
                .map(|r| r.manifest_json);
            Ok((manifest, preferences::skin_choice(conn, &who)?.skin_id))
        })
        .await
        .map_err(|e| failed("skin not chosen", e))?;
    let manifest = manifest.ok_or_else(|| not_found(&id))?;
    let m: SkinManifest =
        serde_json::from_str(&manifest).map_err(|e| failed("skin manifest", e))?;
    let previous = ctx
        .skin
        .trial
        .current(&owner)
        .map_or(stored, |t| t.previous);
    let trial = SkinTrial {
        id,
        name: m.name,
        previous,
        reverts_at_ms: ctx.clock.now_ms().saturating_add(SKIN_TRIAL_MS),
    };
    let wait = Duration::from_millis(SKIN_TRIAL_MS);
    let seq = ctx.skin.trial.begin(owner, trial.clone());
    serve::announce(ctx).await?;
    publish(ctx, Some(trial));
    let host = ctx.this.upgrade().expect("a context in use is alive");
    ctx.unlock_scope_or_closed()
        .spawn_or_drop("skin trial revert", async move {
            tokio::time::sleep(wait).await;
            if host.skin.trial.take_if(|h| h.seq == seq).is_some() {
                revert_announced(&host).await;
            }
        });
    Ok(())
}

/// Tell every window the stored choice is back and the trial is over.
async fn revert_announced(ctx: &DaemonContext) {
    if let Err(refused) = serve::announce(ctx).await {
        tracing::warn!(?refused, "the reverted skin was not announced");
    }
    publish(ctx, None);
}

/// A window could not apply `id` (`SkinApplyFailed`): if it is `owner`'s
/// trial, end it now and put the stored choice back, so a skin that does
/// not render is never left on screen until the timer. True if reverted.
pub(super) async fn revert_failed(ctx: &DaemonContext, owner: &str, id: &str) -> bool {
    let ended = ctx
        .skin
        .trial
        .take_if(|h| h.owner == owner && h.trial.id == id)
        .is_some();
    if ended {
        revert_announced(ctx).await;
    }
    ended
}

/// End `owner`'s trial because another choice was stored (a built-in,
/// or the stored skin chosen again).
pub(super) fn end(ctx: &DaemonContext, owner: &str) {
    if ctx.skin.trial.take_if(|h| h.owner == owner).is_some() {
        publish(ctx, None);
    }
}

/// `SkinKeep`: settle the trial of `id` now.
pub(super) async fn keep(ctx: &DaemonContext, id: &str, keep: bool) -> Answer<()> {
    let (db, owner) = (ctx.require_db()?, ctx.require_owner()?);
    let trial = ctx
        .skin
        .trial
        .take_if(|h| h.owner == owner && h.trial.id == id)
        .ok_or_else(|| {
            rekindle_ipc::protocol::IpcResponse::error(409, format!("{id} is not on trial"))
        })?;
    if !keep {
        revert_announced(ctx).await;
        return Ok(());
    }
    let skin_id = trial.id.clone();
    let kept = db
        .call(move |conn| preferences::set_skin_id(conn, &owner, &skin_id))
        .await;
    if let Err(e) = kept {
        // Not stored: the windows go back to the stored choice.
        revert_announced(ctx).await;
        return Err(failed("skin not kept", e));
    }
    publish(ctx, None);
    Ok(())
}
