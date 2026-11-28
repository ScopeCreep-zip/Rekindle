//! Settle the legacy signaling interpreter’s invite row.

use std::sync::Arc;

use crate::state::AppState;

/// Settle `call_id`'s invite row (`answered`, or discarded). A storage
/// failure is logged: the row then fails at its deadline, the invite's
/// expiry.
pub(super) async fn settle_invite(state: &Arc<AppState>, call_id: &str, answered: bool) {
    let host = match crate::state_helpers::friend_host(state) {
        Ok(host) => host,
        Err(e) => {
            tracing::debug!(call = %call_id, error = %e, "invite row not settled: no host");
            return;
        }
    };
    let settled =
        rekindle_transport::operations::dm::call_invite::settle(host.as_ref(), call_id, answered)
            .await;
    if let Err(e) = settled {
        tracing::warn!(call = %call_id, error = %e,
            "invite row not settled; it fails at the invite's expiry");
    }
}
