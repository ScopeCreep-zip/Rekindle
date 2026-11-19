//! The unlocked identity's host state, one implementation for the daemon's
//! `Unlock` and the GUI's login (plan D3.1): what an unlock installs (the
//! signing key, the session projection, the MEKs,
//! the audit chain, the trusted clients, the friend authority) and what a
//! lock takes out again.

use zeroize::Zeroizing;

use super::identity::Unlocked;
use crate::daemon::dispatch::DaemonContext;

/// What an install hands the caller.
pub struct Installed {
    pub public_key: String,
    pub display_name: String,
    /// The Ed25519 identity secret.
    pub secret: Zeroizing<[u8; 32]>,
    /// What the audit chain's restore found, when it ran.
    pub audit: Option<rekindle_audit::Restored>,
}

/// Install `unlocked` (its vault and database are in the host's stores
/// already, `host::identity::{create, unlock}`).
///
/// # Errors
/// The session projection cannot be loaded; everything installed is taken
/// out again.
pub(crate) async fn install(ctx: &DaemonContext, unlocked: Unlocked) -> Result<Installed, String> {
    let Unlocked {
        public_key,
        display_name,
        secret,
        friend_store,
        failed_attempts,
    } = unlocked;
    // This unlock's events start a new journal epoch (plan D3.2).
    ctx.journal.begin();
    *ctx.signing_key.write() = Some(crate::state::signing_key::SigningKeyHandle::new(*secret));
    *ctx.friend_store.write() = Some(friend_store);

    // The session is projected from the identity's stores (plan D1).
    let loaded = match ctx.db.current() {
        Ok(db) => super::session::load(
            &db,
            &ctx.vault,
            &public_key,
            ctx.rng.as_ref(),
            ctx.clock.now_ms_i64(),
        )
        .await
        .map_err(|e| format!("session: {e}")),
        Err(e) => Err(format!("session: {e}")),
    };
    let session = match loaded {
        Ok(session) => session,
        Err(e) => {
            release(ctx).await;
            return Err(e);
        }
    };
    *ctx.session.write() = Some(session);
    let audit = restore_audit(ctx, &public_key, failed_attempts).await;
    // The identity's trusted clients get their clearance back (plan D1).
    crate::daemon::clients::load(ctx).await;
    raise_memory_notice(ctx, &held_secrets_locked(ctx));
    Ok(Installed {
        public_key,
        display_name,
        secret,
        audit,
    })
}

/// Raise [`memory_notice`] for `locked`, standing: it is raised during the
/// install, before any frontend has connected, and holds for the session,
/// so each frontend that connects later is shown it (plan E7.1).
fn raise_memory_notice(ctx: &DaemonContext, locked: &[bool]) {
    if let Some(notice) = memory_notice(locked) {
        ctx.publish_standing(notice);
    }
}

/// Whether each secret the unlocked identity holds for the session (the
/// vault's entry key, the signing key) sits in locked pages.
fn held_secrets_locked(ctx: &DaemonContext) -> [bool; 2] {
    let vault = ctx
        .vault
        .lock()
        .as_ref()
        .is_none_or(rekindle_vault::VaultStore::entry_key_locked);
    let signing = ctx
        .signing_key
        .read()
        .as_ref()
        .is_none_or(crate::state::signing_key::SigningKeyHandle::is_locked);
    [vault, signing]
}

/// The notice an unlock raises once when the OS refused to lock any held
/// secret (plan E7.1: "A failed lock is logged and the key still loads";
/// the user is told too, as GnuPG warns of insecure memory): the key may
/// reach swap, and raising the lock limit (`ulimit -l`) fixes it.
pub(crate) fn memory_notice(
    locked: &[bool],
) -> Option<rekindle_types::subscription_events::SubscriptionEvent> {
    use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};
    let refused = locked.iter().filter(|l| !**l).count();
    (refused > 0).then(|| {
        tracing::warn!(
            refused,
            "the unlocked identity's keys are not in locked memory"
        );
        SubscriptionEvent::Notification(NotificationEvent::SystemAlert {
            title: "Keys not in locked memory".into(),
            subject: None,
            body: "The system refused to lock this identity's keys in memory, so they may be \
                   written to swap. Rekindle keeps running; raise the locked-memory limit \
                   (ulimit -l) to protect them."
                .into(),
        })
    })
}

/// Take the unlocked identity out, last-installed first: its clients'
/// clearance, the cached channel keys, the signing key (zeroized on drop),
/// the audit chain, the session, then the database and
/// vault (plan D1).
pub(crate) async fn release(ctx: &DaemonContext) {
    // Every action held for this identity goes with it (plan D3.3).
    if let Ok(owner) = ctx.require_owner() {
        ctx.intents.clear_matching(|held| held.identity == owner);
        ctx.undo.clear_matching(|held| held.identity == owner);
    }
    // A skin on trial was never stored: the next unlock shows the choice.
    ctx.skin_trial.clear();
    crate::daemon::clients::unload(ctx).await;
    ctx.mek_cache.write().clear();
    *ctx.signing_key.write() = None;
    super::audit::close(&ctx.audit_chain);
    *ctx.session.write() = None;
    // The 1:1 receive state is the identity's, and so are the probes and
    // the profile and prekey leases (their pool ends with the unlock).
    *ctx.friend_store.write() = None;
    ctx.friend_stores.clear();
    *ctx.own_profile_lease.lock() = None;
    *ctx.own_prekey_lease.lock() = None;
    if let Some(transport) = ctx.transport.read().as_ref() {
        transport.relay().clear_probes();
    }
    super::identity::lock(&ctx.identity_stores()).await;
    // No event of this session replays into the next (plan D3.2).
    ctx.journal.begin();
}

/// Open the identity's audit chain and record the wrong passphrases the
/// unlock cleared; what the restore found, when it ran.
async fn restore_audit(
    ctx: &DaemonContext,
    owner: &str,
    failures: u32,
) -> Option<rekindle_audit::Restored> {
    let db = ctx.db.current().ok()?;
    match super::audit::restore(&ctx.audit_chain, &db, &ctx.vault, owner).await {
        Ok(restored) => {
            if let rekindle_audit::Restored::Truncated { anchor_cursor } = restored {
                tracing::error!(anchor_cursor, "audit chain truncated between sessions");
            }
            super::audit::record_unlock_failures(
                &ctx.audit_chain,
                &db,
                &ctx.vault,
                owner,
                failures,
                ctx.clock.now_ms_i64(),
            );
            Some(restored)
        }
        Err(e) => {
            tracing::warn!(error = %e, "audit chain not restored");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent};

    use super::memory_notice;

    #[test]
    fn every_secret_locked_raises_no_notice() {
        assert!(memory_notice(&[true, true]).is_none());
    }

    #[test]
    fn a_refused_lock_raises_one_system_alert() {
        let Some(SubscriptionEvent::Notification(NotificationEvent::SystemAlert {
            title,
            body,
            subject,
        })) = memory_notice(&[true, false])
        else {
            panic!("a refused lock is shown to the user");
        };
        assert_eq!(title, "Keys not in locked memory");
        assert!(body.contains("ulimit -l"), "{body}");
        assert!(subject.is_none());
        assert!(memory_notice(&[false, false]).is_some());
    }

    fn is_memory_notice(event: &SubscriptionEvent) -> bool {
        matches!(
            event,
            SubscriptionEvent::Notification(NotificationEvent::SystemAlert { title, .. })
                if title == "Keys not in locked memory"
        )
    }

    /// The notice is raised while the session installs, before any frontend
    /// has subscribed; both modes' late subscribers still get it.
    #[tokio::test]
    async fn the_memory_notice_reaches_a_frontend_that_connects_after_the_unlock() {
        use rekindle_types::subscription_events::StreamFrame;
        let dir = tempfile::tempdir().unwrap();
        let (host, _) = crate::host::gate::tests::host_with_identity(&dir).await;
        host.ctx.journal.begin();
        super::raise_memory_notice(&host.ctx, &[true, false]);
        // The daemon's bus connection, subscribing with no cursor.
        let mut connected = host.ctx.journal.connect(None);
        let frame = tokio::time::timeout(std::time::Duration::from_secs(5), connected.next())
            .await
            .unwrap();
        assert!(
            matches!(&frame, Some(StreamFrame::Event(s)) if is_memory_notice(&s.event)),
            "{frame:?}"
        );
        // The GUI's window that just opened (`subscribe_events`, no cursor).
        let standing = host.standing();
        assert_eq!(standing.len(), 1);
        assert!(is_memory_notice(&standing[0].event));
        // The next session starts without it.
        host.ctx.journal.begin();
        assert!(host.standing().is_empty());
    }
}
