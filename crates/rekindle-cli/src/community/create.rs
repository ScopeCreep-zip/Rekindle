//! `rekindle community create` — create a new community.

use anyhow::Context;
use tracing::info;

use rekindle_transport::operations::community;
use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Create a new community with a default #general channel.
///
/// Steps:
/// 1. Validate community name
/// 2. Call transport `create_community` operation
/// 3. Update session with new membership
/// 4. Save session to disk
/// 5. Display result
pub async fn cmd_create(
    handle: &TransportHandle,
    session: &Session,
    name: &str,
    description: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let name = helpers::validate_name(name, "Community")?;

    format::step_header(1, 3, "Creating community")?;

    let _signing_key = crate::identity::keystore::load_signing_key().await?;

    let result = community::create_community(
        handle.node(),
        session,
        &name,
        description,
        &handle.mek_cache,
    )
    .await
    .context("community creation failed")?;

    info!(
        governance = %result.governance_key,
        registry = %result.registry_key,
        "community created"
    );

    format::step_done(&format!("community '{name}' created"))?;

    // Step 2: Update session
    format::step_header(2, 3, "Updating session")?;

    let membership = rekindle_transport::CommunityMembership {
        governance_key: result.governance_key.clone(),
        pseudonym_key: result.our_pseudonym_key.clone(),
        display_name: session.identity.display_name.clone(),
        role_ids: vec![0], // owner
        registry_key: result.registry_key.clone(),
        slot_index: result.our_slot_index,
        community_name: name.clone(),
        slot_seed: None, // stored in keyring
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
                "status": "created",
                "name": name,
                "governance_key": result.governance_key,
                "registry_key": result.registry_key,
                "default_channel": result.default_channel_id,
                "pseudonym_key": result.our_pseudonym_key,
                "mek_generation": result.mek_generation,
            }),
            mode,
        )
    } else {
        format::print_text(&format!("\nCommunity '{name}' created."))?;
        format::print_text(&format!(
            "  Governance: {}",
            helpers::abbreviate_key(&result.governance_key)
        ))?;
        format::print_text(&format!(
            "  Registry:   {}",
            helpers::abbreviate_key(&result.registry_key)
        ))?;
        format::print_text(&format!(
            "  Channel:    #general ({})",
            helpers::abbreviate_key(&result.default_channel_id)
        ))?;
        format::print_text("\nNext: invite others with `rekindle community invite create`")
    }
}
