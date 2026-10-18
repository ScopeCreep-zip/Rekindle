//! The tamper signal of the identity's audit log (plan E1.1), both hosts:
//! `SystemEvent::AuditChainBroken` (E6.3 renders it) and a system alert
//! until then, raised by the unlock's audit step and by `AuditVerify`.

use rekindle_audit::Verified;
use rekindle_db::Db;
use rekindle_types::subscription_events::{NotificationEvent, SubscriptionEvent, SystemEvent};

use crate::daemon::dispatch::DaemonContext;

/// The restore at unlock found the chain cut short between sessions.
pub(crate) fn truncated(ctx: &DaemonContext, anchor_cursor: u64) {
    raise(
        ctx,
        anchor_cursor,
        format!(
            "Your device's tamper-evident audit log was modified between sessions. \
             The most recent vault-anchored entry was #{anchor_cursor}, but the \
             on-disk log has different content. Someone with write access to your \
             local database may have removed or altered audit history."
        ),
    );
}

/// Verify the chain end to end; a broken one raises the signal.
pub(crate) async fn verify(ctx: &DaemonContext, db: &Db, owner: &str) -> Verified {
    let verified = super::audit::verify(&ctx.audit_chain, db, owner).await;
    if let Verified::Broken {
        at: Some(cursor),
        ref detail,
        ..
    } = verified
    {
        tracing::error!(owner, cursor, %detail, "audit chain verification failed");
        raise(
            ctx,
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

fn raise(ctx: &DaemonContext, cursor: u64, body: String) {
    ctx.publish_event(SubscriptionEvent::System(SystemEvent::AuditChainBroken {
        cursor,
    }));
    ctx.publish_event(SubscriptionEvent::Notification(
        NotificationEvent::SystemAlert {
            title: "Audit chain broken".into(),
            body,
        },
    ));
}
