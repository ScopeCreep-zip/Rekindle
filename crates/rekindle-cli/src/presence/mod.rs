//! Presence management commands — set status, watch presence.

use anyhow::Context;

use rekindle_transport::operations::presence;
use rekindle_transport::Session;

use crate::cli::PresenceCmd;
use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::{CliEvent, TransportHandle};

/// Dispatch `rekindle presence <subcommand>`.
pub async fn dispatch(
    cmd: &PresenceCmd,
    handle: &TransportHandle,
    session: &Session,
    mode: OutputMode,
) -> anyhow::Result<()> {
    match cmd {
        PresenceCmd::Set {
            status,
            message,
            game,
        } => cmd_set(handle, session, status, message.as_deref(), game.as_deref(), mode).await,
        PresenceCmd::Watch { community } => {
            cmd_watch(handle, session, community.as_deref(), mode).await
        }
    }
}

/// Set presence status.
///
/// Updates the profile DHT record with the new status byte and optional
/// status message. Valid statuses: online, away, busy, invisible.
async fn cmd_set(
    handle: &TransportHandle,
    session: &Session,
    status: &str,
    message: Option<&str>,
    game: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let valid = ["online", "away", "busy", "invisible"];
    if !valid.contains(&status) {
        anyhow::bail!(
            "invalid status '{}' — must be one of: {}",
            status,
            valid.join(", ")
        );
    }

    presence::set_status(handle.node(), session, status, message)
        .await
        .context("failed to set presence")?;

    if let Some(game_name) = game {
        presence::set_game_presence(
            handle.node(),
            session,
            game_name,
            None,
            0,
            None,
        )
        .await
        .context("failed to set game presence")?;
    }

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": status,
                "message": message,
                "game": game,
            }),
            mode,
        )
    } else {
        let msg = message
            .map(|m| format!(" — {m}"))
            .unwrap_or_default();
        let game_str = game
            .map(|g| format!(" (playing {g})"))
            .unwrap_or_default();
        format::print_text(&format!("Status set to {status}{msg}{game_str}."))
    }
}

/// Watch presence updates from friends and community members.
///
/// Subscribes to the transport event channel and filters for DM presence
/// updates and gossip presence updates. Runs until Ctrl+C.
async fn cmd_watch(
    handle: &TransportHandle,
    session: &Session,
    community_filter: Option<&str>,
    mode: OutputMode,
) -> anyhow::Result<()> {
    if !mode.is_structured() {
        let filter_msg = community_filter
            .map(|c| format!(" in '{c}'"))
            .unwrap_or_default();
        format::print_text(&format!(
            "Watching presence updates{filter_msg}... (Ctrl+C to stop)"
        ))?;
    }

    let community_gov_key = community_filter
        .map(|c| helpers::resolve_community(c, session).map(|m| m.governance_key.clone()))
        .transpose()?;

    let mut event_rx = handle.event_rx.lock().await;

    loop {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                match event {
                    CliEvent::Dm {
                        sender_key,
                        sender_name,
                        payload: rekindle_transport::payload::dm::DmPayload::PresenceUpdate { status, game_info },
                        timestamp,
                    } => {
                        let name = helpers::sanitize_for_display(&sender_name);
                        let game = game_info.as_ref().map(|g| g.game_name.as_str());

                        if mode.is_structured() {
                            let _ = format::print_jsonl(&serde_json::json!({
                                "event": "presence",
                                "source": "dm",
                                "peer_key": sender_key,
                                "peer_name": name,
                                "status": status,
                                "game": game,
                                "timestamp": timestamp,
                            }));
                        } else {
                            let game_str = game.map(|g| format!(" (playing {g})")).unwrap_or_default();
                            let _ = format::print_text(&format!(
                                "  {name}: status={status}{game_str}"
                            ));
                        }
                    }
                    CliEvent::Gossip {
                        community_id,
                        sender_pseudonym: _,
                        payload: rekindle_transport::payload::gossip::GossipPayload::PresenceUpdate {
                            pseudonym_key,
                            status,
                            game_name,
                            ..
                        },
                        lamport_ts: _,
                    } => {
                        // Apply community filter
                        if let Some(ref filter_key) = community_gov_key {
                            if community_id != *filter_key {
                                continue;
                            }
                        }

                        let name = helpers::abbreviate_key(&pseudonym_key);

                        if mode.is_structured() {
                            let _ = format::print_jsonl(&serde_json::json!({
                                "event": "presence",
                                "source": "community",
                                "community_id": community_id,
                                "pseudonym_key": pseudonym_key,
                                "status": status,
                                "game": game_name,
                            }));
                        } else {
                            let game_str = game_name.as_ref().map(|g| format!(" (playing {g})")).unwrap_or_default();
                            let _ = format::print_text(&format!(
                                "  {name}: {status}{game_str}"
                            ));
                        }
                    }
                    _ => {}
                }
            }
            _ = tokio::signal::ctrl_c() => {
                if !mode.is_structured() {
                    let _ = format::print_text("\nStopped watching.");
                }
                break;
            }
        }
    }

    Ok(())
}
