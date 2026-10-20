//! The unlocked identity's host state, one implementation for the daemon's
//! `Unlock` and the GUI's login (plan D3.1): what an unlock installs (the
//! signing key, the session projection, the MEKs,
//! the audit chain, the trusted clients, the friend authority) and what a
//! lock takes out again.

use std::sync::Arc;

use rekindle_db::SqliteFriendStore;
use zeroize::Zeroizing;

use super::identity::Unlocked;
use crate::daemon::dispatch::DaemonContext;

/// What an install hands the caller.
pub struct Installed {
    pub public_key: String,
    pub display_name: String,
    /// The Ed25519 identity secret.
    pub secret: Zeroizing<[u8; 32]>,
    /// The receive path's friend authority over the installed database.
    pub friend_store: Arc<SqliteFriendStore>,
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
    *ctx.friend_store.write() = Some(Arc::clone(&friend_store));

    // The session is projected from the identity's stores (plan D1).
    let loaded = match ctx.db.current() {
        Ok(db) => super::session::load(&db, &ctx.vault, &public_key)
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
    crate::daemon::mek_rotation::restore_from_vault(ctx);
    let audit = restore_audit(ctx, &public_key, failed_attempts).await;
    // The identity's trusted clients get their clearance back (plan D1).
    crate::daemon::clients::load(ctx).await;
    Ok(Installed {
        public_key,
        display_name,
        secret,
        friend_store,
        audit,
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
