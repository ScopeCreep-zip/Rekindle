use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{Emitter, State};
use tokio::sync::mpsc;

use crate::channels::VoiceEvent;
use crate::state::{SharedState, VoiceEngineHandle};

/// Join a voice channel — initialize the voice engine and emit join event.
#[tauri::command]
pub async fn join_voice_channel(
    channel_id: String,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let identity = state
        .identity
        .read()
        .clone()
        .ok_or("not logged in")?;

    // Initialize voice engine if not already active
    {
        let mut ve = state.voice_engine.lock();
        if ve.is_none() {
            let config = rekindle_voice::VoiceConfig::default();
            let engine = rekindle_voice::VoiceEngine::new(config)
                .map_err(|e| format!("failed to create voice engine: {e}"))?;
            *ve = Some(VoiceEngineHandle {
                engine,
                send_loop_shutdown: None,
                send_loop_handle: None,
            });
        }
    }

    // Start audio capture and playback
    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle
                .engine
                .start_capture()
                .map_err(|e| format!("failed to start capture: {e}"))?;
            handle
                .engine
                .start_playback()
                .map_err(|e| format!("failed to start playback: {e}"))?;
        }
    }

    // Create voice transport for this channel and attempt to connect.
    // For DM voice calls, channel_id is the peer's public key — look up their route.
    // For community voice channels, the route comes from the channel's DHT record.
    let mut transport = rekindle_voice::transport::VoiceTransport::new(channel_id.clone());

    // Try to look up a route blob for this channel (peer key or community channel)
    let route_blob = {
        let dht_mgr = state.dht_manager.read();
        dht_mgr
            .as_ref()
            .and_then(|mgr| mgr.manager.get_cached_route(&channel_id).cloned())
    };

    // Clone API handle out before await (parking_lot guards are !Send)
    let api = {
        let node = state.node.read();
        node.as_ref().map(|nh| nh.api.clone())
    };

    // If we have both a route blob and API, connect the transport now
    if let (Some(blob), Some(api)) = (route_blob, api) {
        let sender_key = hex::decode(&identity.public_key).unwrap_or_default();
        if let Err(e) = transport.connect(api, &blob, sender_key) {
            tracing::warn!(error = %e, channel = %channel_id, "voice transport connect failed — audio only local");
        }
    }

    // Take capture_rx and config from the engine for the send loop task
    let capture_rx = {
        let mut ve = state.voice_engine.lock();
        ve.as_mut().and_then(|handle| handle.engine.take_capture_rx())
    };

    // Spawn voice send loop: drain capture_rx -> VAD -> Opus encode -> transport send
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);
    let send_app = app.clone();
    let send_public_key = identity.public_key.clone();

    let send_handle = tokio::spawn(voice_send_loop(
        capture_rx,
        transport,
        shutdown_rx,
        send_app,
        send_public_key,
    ));

    // Store shutdown handle and join handle on the engine
    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.send_loop_shutdown = Some(shutdown_tx);
            handle.send_loop_handle = Some(send_handle);
        }
    }

    // Emit voice events to frontend
    let event = VoiceEvent::UserJoined {
        public_key: identity.public_key.clone(),
        display_name: identity.display_name,
    };
    let _ = app.emit("voice-event", &event);

    // Report initial connection quality
    let quality_event = VoiceEvent::ConnectionQuality {
        quality: "good".to_string(),
    };
    let _ = app.emit("voice-event", &quality_event);

    // Report speaking state (initially not speaking)
    let speaking_event = VoiceEvent::UserSpeaking {
        public_key: identity.public_key,
        speaking: false,
    };
    let _ = app.emit("voice-event", &speaking_event);

    Ok(())
}

/// Leave the current voice channel.
#[tauri::command]
pub async fn leave_voice(
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let public_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();

    // Signal the send loop to shut down and wait for it to finish.
    // Extract the shutdown sender and join handle outside the lock,
    // then signal and await outside (parking_lot guards are !Send).
    let (shutdown_tx, send_handle) = {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            (
                handle.send_loop_shutdown.take(),
                handle.send_loop_handle.take(),
            )
        } else {
            (None, None)
        }
    };

    // Signal shutdown — dropping the sender also works, but explicit is clearer
    if let Some(tx) = shutdown_tx {
        let _ = tx.send(()).await;
    }

    // Wait for the send loop to finish (it will disconnect transport internally)
    if let Some(handle) = send_handle {
        let _ = handle.await;
    }

    // Stop audio and clean up
    {
        let mut ve = state.voice_engine.lock();
        if let Some(ref mut handle) = *ve {
            handle.engine.stop_capture();
            handle.engine.stop_playback();
        }
    }
    *state.voice_engine.lock() = None;

    // Emit leave event
    let event = VoiceEvent::UserLeft { public_key };
    let _ = app.emit("voice-event", &event);

    Ok(())
}

/// Set microphone mute state.
#[tauri::command]
pub async fn set_mute(
    muted: bool,
    app: tauri::AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    if let Some(ref mut handle) = *state.voice_engine.lock() {
        handle.engine.set_muted(muted);
    }

    let public_key = state
        .identity
        .read()
        .as_ref()
        .map(|id| id.public_key.clone())
        .unwrap_or_default();

    let event = VoiceEvent::UserMuted { public_key, muted };
    let _ = app.emit("voice-event", &event);

    Ok(())
}

/// Set deafen state (mute all audio output).
#[tauri::command]
pub async fn set_deafen(deafened: bool, state: State<'_, SharedState>) -> Result<(), String> {
    if let Some(ref mut handle) = *state.voice_engine.lock() {
        handle.engine.set_deafened(deafened);
    }
    Ok(())
}

/// Voice send loop: drains `capture_rx`, runs VAD, encodes with Opus, sends via transport.
///
/// This task owns the `VoiceTransport` and runs until a shutdown signal is received
/// or the capture channel closes. On exit it disconnects the transport.
async fn voice_send_loop(
    capture_rx: Option<mpsc::Receiver<Vec<f32>>>,
    mut transport: rekindle_voice::transport::VoiceTransport,
    mut shutdown_rx: mpsc::Receiver<()>,
    app: tauri::AppHandle,
    public_key: String,
) {
    let Some(mut capture_rx) = capture_rx else {
        tracing::warn!("voice send loop started without capture_rx — exiting");
        return;
    };

    // Create a dedicated codec and VAD for this task (the engine's instances
    // live behind a parking_lot Mutex which is !Send across .await points).
    let sample_rate: u32 = 48000;
    let channels: u16 = 1;
    let frame_size: usize = 960; // 20ms at 48kHz

    let mut codec = match rekindle_voice::codec::OpusCodec::new(sample_rate, channels, frame_size) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "voice send loop: failed to create Opus codec");
            return;
        }
    };

    let frame_duration_ms = u32::try_from(frame_size).unwrap_or(960) * 1000 / sample_rate;
    let mut vad = rekindle_voice::vad::VoiceActivityDetector::new(0.02, 300, frame_duration_ms);

    let mut sequence: u32 = 0;
    let mut was_speaking = false;

    // Accumulation buffer: cpal may deliver chunks that don't align with
    // Opus frame boundaries, so we accumulate samples here.
    let mut pcm_buffer: Vec<f32> = Vec::with_capacity(frame_size * 2);

    tracing::info!("voice send loop started");

    loop {
        tokio::select! {
            biased;

            // Shutdown signal takes priority
            _ = shutdown_rx.recv() => {
                tracing::info!("voice send loop: shutdown signal received");
                break;
            }

            // Receive PCM samples from the audio capture callback
            maybe_samples = capture_rx.recv() => {
                let Some(samples) = maybe_samples else {
                    tracing::info!("voice send loop: capture channel closed");
                    break;
                };

                pcm_buffer.extend_from_slice(&samples);

                // Process all complete frames in the buffer
                while pcm_buffer.len() >= frame_size {
                    let frame_samples: Vec<f32> =
                        pcm_buffer.drain(..frame_size).collect();

                    // Run VAD on this frame
                    let speaking = vad.process(&frame_samples);

                    // Emit speaking state change to frontend
                    if speaking != was_speaking {
                        was_speaking = speaking;
                        let event = VoiceEvent::UserSpeaking {
                            public_key: public_key.clone(),
                            speaking,
                        };
                        let _ = app.emit("voice-event", &event);
                    }

                    // Only encode and send if speaking (VAD gate)
                    if !speaking {
                        continue;
                    }

                    // Encode the PCM frame with Opus
                    let mut encoded = match codec.encode(&frame_samples) {
                        Ok(frame) => frame,
                        Err(e) => {
                            tracing::warn!(error = %e, "voice send loop: Opus encode failed");
                            continue;
                        }
                    };

                    // Fill in sequence and timestamp
                    encoded.sequence = sequence;
                    encoded.timestamp = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
                        .unwrap_or(0);
                    sequence = sequence.wrapping_add(1);

                    // Send via transport (only if connected)
                    if transport.is_connected() {
                        if let Err(e) = transport.send(&encoded).await {
                            tracing::debug!(error = %e, "voice send loop: transport send failed");
                        }
                    }
                }
            }
        }
    }

    // Clean up: disconnect transport
    if let Err(e) = transport.disconnect() {
        tracing::warn!(error = %e, "voice send loop: transport disconnect failed");
    }

    // Emit final not-speaking state
    if was_speaking {
        let event = VoiceEvent::UserSpeaking {
            public_key,
            speaking: false,
        };
        let _ = app.emit("voice-event", &event);
    }

    tracing::info!("voice send loop exited");
}
