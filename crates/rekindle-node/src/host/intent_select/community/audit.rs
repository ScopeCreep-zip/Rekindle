//! The selection of a audit and notification default request (plans D3.3, E3.1).

use super::{community, members, AuditRequest, DaemonContext, IpcResponse, Selection, Subject};

/// `selection` for the audit and notification default requests.
pub(super) fn selection(
    ctx: &DaemonContext,
    request: &AuditRequest,
    s: &mut Selection,
) -> Result<(), IpcResponse> {
    if let AuditRequest::NotificationDefaultSet {
        community: target, ..
    } = request
    {
        let (id, name) = community(ctx, target)?;

        if matches!(request, AuditRequest::NotificationDefaultSet { .. }) {
            s.notifies = members(ctx, &id);
        }
        s.subject = Subject::Community { id, name };
    }
    Ok(())
}
