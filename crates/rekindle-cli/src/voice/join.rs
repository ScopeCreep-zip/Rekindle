//! `rekindle voice join` — join a voice channel.

use anyhow::Context;

use rekindle_transport::operations::voice;
use rekindle_transport::Session;

use crate::helpers;
use crate::output::format;
use crate::output::OutputMode;
use crate::transport::TransportHandle;

/// Join a voice channel and enter the voice session.
///
/// In CLI mode, this runs in the foreground until Ctrl+C. The voice
/// session key is derived from the channel's MEK, a route is allocated
/// for voice packets, and a `VoiceJoin` gossip is broadcast to the
/// community mesh. Other participants' voice packets arrive via the
/// transport event channel.
///
/// In TUI mode (M2), this transitions to the VoiceSession view which
/// handles the audio capture/playback loop.
#[allow(clippy::print_stderr)]
pub async fn cmd_join(
    handle: &TransportHandle,
    session: &Session,
    community_ref: &str,
    channel_ref: &str,
    muted: bool,
    deafened: bool,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let membership = helpers::resolve_community(community_ref, session)?;
    let channel_id = helpers::resolve_channel_id(channel_ref);

    format::print_text(&format!(
        "Joining voice channel #{channel_ref} in '{}'...",
        membership.community_name
    ))?;

    let voice_session = voice::join_voice(
        handle.node(),
        membership,
        &channel_id,
        &handle.mek_cache,
        muted,
        deafened,
    )
    .await
    .context("failed to join voice channel")?;

    if mode.is_structured() {
        format::print_structured(
            &serde_json::json!({
                "status": "joined",
                "community": membership.community_name,
                "channel": channel_ref,
                "muted": voice_session.muted,
                "deafened": voice_session.deafened,
            }),
            mode,
        )?;
    } else {
        format::print_text(&format!(
            "Joined voice channel #{channel_ref}.",
        ))?;
        if voice_session.muted {
            format::print_text("  You are muted.")?;
        }
        if voice_session.deafened {
            format::print_text("  You are deafened.")?;
        }
        format::print_text("")?;
        format::print_text("Press Ctrl+C to leave the voice session.")?;
    }

    // Run voice session event loop — drain transport events until Ctrl+C
    let mut event_rx = handle.event_rx.lock().await;
    let mut packet_count: u64 = 0;

    loop {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                match event {
                    crate::transport::CliEvent::VoicePacket { sender_key } => {
                        packet_count += 1;
                        if packet_count % 100 == 1 && !mode.is_structured() {
                            // Log every 100th packet to show activity without flooding
                            let key_short = helpers::abbreviate_key(&sender_key);
                            eprintln!("  [voice] packet from {key_short} (total: {packet_count})");
                        }
                    }
                    crate::transport::CliEvent::Gossip {
                        payload: rekindle_transport::payload::gossip::GossipPayload::Control(
                            rekindle_transport::payload::gossip::ControlPayload::VoiceLeave { channel_id: ref leave_ch }
                        ),
                        sender_pseudonym,
                        ..
                    } if *leave_ch == channel_id => {
                        if !mode.is_structured() {
                            format::print_text(&format!(
                                "  {} left the voice channel.",
                                helpers::abbreviate_key(&sender_pseudonym)
                            ))?;
                        }
                    }
                    crate::transport::CliEvent::Gossip {
                        payload: rekindle_transport::payload::gossip::GossipPayload::Control(
                            rekindle_transport::payload::gossip::ControlPayload::VoiceJoin { channel_id: ref join_ch, .. }
                        ),
                        sender_pseudonym,
                        ..
                    } if *join_ch == channel_id => {
                        if !mode.is_structured() {
                            format::print_text(&format!(
                                "  {} joined the voice channel.",
                                helpers::abbreviate_key(&sender_pseudonym)
                            ))?;
                        }
                    }
                    _ => {}
                }
            }
            _ = tokio::signal::ctrl_c() => {
                break;
            }
        }
    }

    // Leave the voice session
    let mut session_copy = voice_session;
    voice::leave_voice(&mut session_copy);

    if !mode.is_structured() {
        format::print_text(&format!(
            "\nLeft voice channel. ({packet_count} packets received)"
        ))?;
    }

    Ok(())
}
