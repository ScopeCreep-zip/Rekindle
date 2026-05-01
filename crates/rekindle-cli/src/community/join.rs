//! `rekindle community join` — join an existing community via invite.

use anyhow::Context;
use tracing::info;

use rekindle_transport::operations::community;
use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Join a community using an invite code or governance key.
///
/// Steps:
/// 1. Resolve invite to governance key
/// 2. Call transport `join_community` operation
/// 3. Update session with new membership
/// 4. Save session to disk
/// 5. Display channels and member count
pub async fn cmd_join(
    handle: &TransportHandle,
    session: &Session,
    invite: &str,
    display_name_override: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    // Resolve the invite — could be a raw governance key or an invite code
    // For now, treat the invite string as the governance key directly.
    // Full invite code resolution (BLAKE3 hash lookup) will be implemented
    // when the invite ceremony is wired through gossip.
    let governance_key = invite.trim();

    if governance_key.is_empty() {
        anyhow::bail!("invite code or governance key is required");
    }

    let display_name = display_name_override
        .map(helpers::validate_display_name)
        .transpose()?
        .unwrap_or_else(|| session.identity.display_name.clone());

    // Check if already a member
    if session.community(governance_key).is_some() {
        anyhow::bail!(
            "already a member of this community\n\
             leave first: rekindle community leave {governance_key}"
        );
    }

    format::step_header(1, 3, "Joining community")?;

    let signing_key = crate::identity::keystore::load_signing_key().await?;

    let result = community::join_community(
        handle.node(),
        session,
        governance_key,
        &display_name,
        &handle.mek_cache,
        &signing_key,
    )
    .await
    .context("join failed")?;

    info!(
        community = %result.community_name,
        governance = %result.governance_key,
        channels = result.channels.len(),
        meks = result.meks_cached,
        "community joined"
    );

    format::step_done(&format!("joined '{}'", result.community_name))?;

    // Step 2: Update session
    format::step_header(2, 3, "Updating session")?;

    let membership = rekindle_transport::CommunityMembership {
        governance_key: result.governance_key.clone(),
        pseudonym_key: result.our_pseudonym_key.clone(),
        display_name: result.display_name.clone(),
        role_ids: Vec::new(),
        registry_key: result.registry_key.clone(),
        slot_index: result.our_slot_index,
        community_name: result.community_name.clone(),
        slot_seed: None,
    };

    let mut updated_session = session.clone();
    updated_session.join_community(membership);

    let session_path = helpers::session_path()?;
    updated_session.save(&session_path)?;

    format::step_done("session saved")?;

    // Step 3: Display result
    format::step_header(3, 3, "Complete")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "joined",
                "community_name": result.community_name,
                "governance_key": result.governance_key,
                "pseudonym_key": result.our_pseudonym_key,
                "display_name": result.display_name,
                "channels": result.channels.len(),
                "meks_cached": result.meks_cached,
            }),
            mode,
        )
    } else {
        format::print_text(&format!("\nJoined '{}'.", result.community_name))?;
        format::print_text(&format!("  {} channels available", result.channels.len()))?;
        format::print_text(&format!("  {} MEKs pre-cached", result.meks_cached))?;

        if !result.channels.is_empty() {
            format::print_text("\nChannels:")?;
            for ch in &result.channels {
                format::print_text(&format!("  #{} ({})", ch.name, ch.kind))?;
            }
        }

        format::print_text("\nSend a message: rekindle channel send <community> <channel> <message>")
    }
}
