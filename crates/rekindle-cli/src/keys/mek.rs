//! MEK (Message Encryption Key) operations — list, rotate, request.

use anyhow::Context;

use rekindle_transport::operations::mek;
use rekindle_transport::Session;

use crate::cli::MekCmd;
use crate::helpers;
use crate::output::format;
use crate::output::table;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Dispatch `rekindle key mek <subcommand>`.
pub async fn dispatch(
    cmd: &MekCmd,
    handle: &TransportHandle,
    session: &Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        MekCmd::List { community } => cmd_list(handle, session, community, mode),
        MekCmd::Rotate { community, channel } => {
            cmd_rotate(handle, session, community, channel, mode).await
        }
        MekCmd::Request { community, channel } => {
            cmd_request(handle, session, community, channel, mode)
        }
    }
}

/// List cached MEKs for a community.
///
/// Reads the in-memory MEK cache (populated on join and by MEK transfer
/// gossip messages) and displays generation, age, and channel for each.
fn cmd_list(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;

    let snapshot = handle
        .mek_cache
        .read()
        .snapshot(&membership.governance_key);

    if mode.is_structured() {
        return format::print_structured(&snapshot, mode);
    }

    if snapshot.is_empty() {
        return format::print_text(&format!(
            "No MEKs cached for '{}'.\n\
             MEKs are received when joining a community or requested from peers.",
            membership.community_name
        ));
    }

    let headers = &["Channel", "Generation", "Age"];
    let rows: Vec<Vec<String>> = snapshot
        .iter()
        .map(|e| {
            let channel = if e.channel_id.is_empty() {
                "(community-wide)".to_string()
            } else {
                e.channel_id.clone()
            };
            vec![
                channel,
                e.generation.to_string(),
                helpers::format_uptime(e.age_secs),
            ]
        })
        .collect();

    format::print_text(&format!(
        "MEK cache for '{}' ({} entries):",
        membership.community_name,
        snapshot.len()
    ))?;
    table::print_table(headers, &rows, mode)
}

/// Force MEK rotation for a channel.
///
/// Generates a new MEK, wraps it for every member via ECDH, writes
/// wrapped copies to the MEK vault in the registry, and returns the
/// generation number. The caller broadcasts a `MekRotated` gossip.
async fn cmd_rotate(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    channel_ref: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;
    let channel_id = helpers::resolve_channel_id(channel_ref);

    let signing_key = crate::identity::keystore::load_signing_key().await?;

    let result = mek::rotate_mek(
        handle.node(),
        membership,
        &channel_id,
        &handle.mek_cache,
        &signing_key,
    )
    .await
    .context("MEK rotation failed")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "rotated",
                "community": membership.community_name,
                "channel": channel_ref,
                "generation": result.generation,
                "copies_written": result.copies_written,
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "MEK rotated for #{channel_ref} in '{}'.\n\
             Generation: {}\n\
             Wrapped for {} members.",
            membership.community_name, result.generation, result.copies_written
        ))
    }
}

/// Request the current MEK from community peers.
///
/// Builds and displays a `RequestMek` gossip payload. The CLI doesn't
/// broadcast gossip directly — the caller signs and broadcasts.
/// In practice, the transport event handler receives `MekTransfer`
/// responses and caches them automatically.
fn cmd_request(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    channel_ref: &str,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;
    let channel_id = helpers::resolve_channel_id(channel_ref);

    // Get the latest known generation (or 0 if none cached)
    let needed_gen = handle
        .mek_cache
        .read()
        .current(&membership.governance_key, &channel_id)
        .map_or(0, |m| m.generation() + 1);

    let _payload = mek::build_mek_request_payload(
        &channel_id,
        needed_gen,
        &membership.pseudonym_key,
    )
    .context("failed to build MEK request")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "requested",
                "community": membership.community_name,
                "channel": channel_ref,
                "needed_generation": needed_gen,
            }),
            mode,
        )
    } else {
        format::print_text(&format!(
            "MEK request sent for #{channel_ref} in '{}' (generation >= {needed_gen}).\n\
             The MEK will be cached automatically when a peer responds.",
            membership.community_name
        ))
    }
}
