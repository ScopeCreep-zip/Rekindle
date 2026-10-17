//! The unlocked identity's host state, one implementation for the daemon's
//! `Unlock` and the GUI's login (plan D3.1): what an unlock installs (the
//! signing key, the one Signal manager, the session projection, the MEKs,
//! the audit chain, the trusted clients) and what a lock takes out again.

use std::sync::Arc;

use rekindle_crypto::signal::SignalSessionManager;
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
    /// The unlock's one Signal manager, shared with the host.
    pub signal: Arc<SignalSessionManager>,
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
        signal,
        friend_store,
        failed_attempts,
    } = unlocked;
    *ctx.signing_key.write() = Some(crate::state::signing_key::SigningKeyHandle::new(*secret));
    let signal = Arc::new(signal);
    *ctx.signal.write() = Some(Arc::clone(&signal));

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
        signal,
        friend_store,
        audit,
    })
}

/// Take the unlocked identity out, last-installed first: its clients'
/// clearance, the cached channel keys, the signing key (zeroized on drop),
/// the audit chain, the session, the Signal manager, then the database and
/// vault (plan D1).
pub(crate) async fn release(ctx: &DaemonContext) {
    crate::daemon::clients::unload(ctx).await;
    ctx.mek_cache.write().clear();
    *ctx.signing_key.write() = None;
    super::audit::close(&ctx.audit_chain);
    *ctx.session.write() = None;
    *ctx.signal.write() = None;
    super::identity::lock(&ctx.identity_stores()).await;
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
            );
            Some(restored)
        }
        Err(e) => {
            tracing::warn!(error = %e, "audit chain not restored");
            None
        }
    }
}
