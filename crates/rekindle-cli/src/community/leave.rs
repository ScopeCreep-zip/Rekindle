//! `rekindle community leave` — leave a community.

use anyhow::Context;
use tracing::info;

use rekindle_transport::operations::community;
use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Leave a community after confirmation.
///
/// Steps:
/// 1. Resolve community from name/key
/// 2. Confirm with user (unless --yes)
/// 3. Call transport `leave_community` operation
/// 4. Remove membership from session
/// 5. Save session
pub async fn cmd_leave(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    skip_confirm: bool,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;
    let community_name = membership.community_name.clone();
    let governance_key = membership.governance_key.clone();

    if !skip_confirm {
        let confirmed = helpers::confirm(&format!(
            "Leave '{community_name}'? You will lose access to all channels and messages."
        ))?;
        if !confirmed {
            format::print_text("Cancelled.")?;
            return Ok(());
        }
    }

    let result = community::leave_community(handle.node(), membership, &handle.mek_cache)
        .await
        .context("leave failed")?;

    info!(
        community = %community_name,
        governance = %governance_key,
        "community left"
    );

    // Update session
    let mut updated_session = session.clone();
    updated_session.leave_community(&governance_key);
    let session_path = helpers::session_path()?;
    updated_session.save(&session_path)?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "left",
                "community_name": community_name,
                "governance_key": governance_key,
                "leave_payload_size": result.leave_payload_bytes.len(),
            }),
            mode,
        )
    } else {
        format::print_text(&format!("Left '{community_name}'."))
    }
}
