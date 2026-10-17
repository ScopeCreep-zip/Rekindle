//! The desktop over `rekindle_node::host::audit` (plan D1): the unlocked
//! identity's chain in `AppState.audit_chain`, and the desktop's tamper
//! signal — `SystemEvent::AuditChainBroken` (E6.3 renders it) and a system
//! alert until then.

use rekindle_audit::{AuditKind, Restored, Verified};
use rekindle_db::Db;

use crate::state::AppState;

/// Append one entry to the unlocked identity's chain.
pub fn append(
    state: &AppState,
    pool: &Db,
    owner: &str,
    kind: AuditKind,
    payload: serde_json::Value,
) {
    rekindle_node::host::audit::append(
        &state.audit_chain,
        pool,
        &state.keystore,
        owner,
        kind,
        payload,
    );
}

/// Raise the tamper signal when the host's restore at unlock found the
/// chain truncated between sessions (`host::unlocked::install`).
pub fn report_restored(app: &tauri::AppHandle, owner: &str, restored: Option<&Restored>) {
    if let Some(Restored::Truncated { anchor_cursor }) = restored {
        let anchor_cursor = *anchor_cursor;
        tracing::error!(
            owner,
            anchor_cursor,
            "audit chain truncated between sessions"
        );
        broken(
            app,
            anchor_cursor,
            format!(
                "Your device's tamper-evident audit log was modified between sessions. \
                 The most recent vault-anchored entry was #{anchor_cursor}, but the \
                 on-disk log has different content. Someone with write access to your \
                 local database may have removed or altered audit history."
            ),
        );
    }
}

/// Verify the chain end to end; a broken one raises the tamper signal.
pub async fn verify(app: &tauri::AppHandle, state: &AppState, pool: &Db, owner: &str) -> Verified {
    let verified = rekindle_node::host::audit::verify(&state.audit_chain, pool, owner).await;
    if let Verified::Broken {
        at: Some(cursor),
        ref detail,
        ..
    } = verified
    {
        tracing::error!(owner, cursor, detail = %detail, "audit chain verification failed");
        broken(
            app,
            cursor,
            format!(
                "Your device's tamper-evident audit log failed integrity check at entry \
                 #{cursor}. Someone with write access to your local database may have \
                 modified history. Verify out-of-band before trusting any post-tamper actions."
            ),
        );
    }
    verified
}

fn broken(app: &tauri::AppHandle, cursor: u64, body: String) {
    crate::event_dispatch::emit_subscription(
        app,
        &rekindle_types::subscription_events::SubscriptionEvent::System(
            rekindle_types::subscription_events::SystemEvent::AuditChainBroken { cursor },
        ),
    );
    crate::event_dispatch::emit_notification(
        app,
        rekindle_types::subscription_events::NotificationEvent::SystemAlert {
            title: "Audit chain broken".into(),
            body,
        },
    );
}
