//! Voice packet receive → decode → mix → playback pipeline.
//!
//! Receives `VoicePacket`s from the dispatch loop, decodes per-participant
//! using Opus (with FEC/PLC fallback), mixes with `AudioMixer`, and sends
//! to the playback channel on a 20ms tick cadence.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::Emitter;
use tokio::sync::{broadcast, mpsc};

use crate::channels::VoiceEvent;

pub(crate) struct VoiceReceiveParams {
    pub packet_rx: mpsc::Receiver<rekindle_voice::transport::VoicePacket>,
    pub playback_tx: Option<mpsc::Sender<Vec<f32>>>,
    pub shutdown_rx: mpsc::Receiver<()>,
    pub app: tauri::AppHandle,
    pub our_public_key: String,
    pub deafened_flag: Arc<AtomicBool>,
    pub speaker_ref_tx: broadcast::Sender<Vec<f32>>,
}

struct ParticipantDecoder {
    codec: rekindle_voice::codec::OpusCodec,
    jitter_buffer: rekindle_voice::jitter::JitterBuffer,
    is_speaking: bool,
    last_packet_time: Instant,
}

struct VoiceReceiveLoop {
    packet_rx: mpsc::Receiver<rekindle_voice::transport::VoicePacket>,
    playback_tx: mpsc::Sender<Vec<f32>>,
    shutdown_rx: mpsc::Receiver<()>,
    app: tauri::AppHandle,
    our_key_bytes: Vec<u8>,
    deafened_flag: Arc<AtomicBool>,
    speaker_ref_tx: broadcast::Sender<Vec<f32>>,
    participants: HashMap<Vec<u8>, ParticipantDecoder>,
    mixer: rekindle_voice::mixer::AudioMixer,
    frame_size: usize,
    sample_rate: u32,
    channels: u16,
    jitter_buffer_ms: u32,
    packets_received: u64,
    last_quality_check: Instant,
}

/// Entry point: validate params, build loop state, run until shutdown.
pub(crate) async fn run(params: VoiceReceiveParams) {
    let Some(loop_state) = VoiceReceiveLoop::new(params) else {
        return;
    };
    loop_state.run_loop().await;
}

impl VoiceReceiveLoop {
    fn new(params: VoiceReceiveParams) -> Option<Self> {
        let Some(playback_tx) = params.playback_tx else {
            tracing::warn!("voice receive loop started without playback_tx — exiting");
            return None;
        };

        let sample_rate: u32 = 48000;
        let channels: u16 = 1;
        let frame_size: usize = 960;
        let jitter_buffer_ms: u32 = 200;

        Some(Self {
            packet_rx: params.packet_rx,
            playback_tx,
            shutdown_rx: params.shutdown_rx,
            app: params.app,
            our_key_bytes: hex::decode(&params.our_public_key).unwrap_or_default(),
            deafened_flag: params.deafened_flag,
            speaker_ref_tx: params.speaker_ref_tx,
            participants: HashMap::new(),
            mixer: rekindle_voice::mixer::AudioMixer::new(channels),
            frame_size,
            sample_rate,
            channels,
            jitter_buffer_ms,
            packets_received: 0,
            last_quality_check: Instant::now(),
        })
    }

    async fn run_loop(mut self) {
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tracing::info!("voice receive loop started");

        loop {
            tokio::select! {
                biased;
                _ = self.shutdown_rx.recv() => {
                    tracing::info!("voice receive loop: shutdown signal received");
                    break;
                }
                Some(packet) = self.packet_rx.recv() => {
                    self.ingest_packet(packet);
                }
                _ = tick.tick() => {
                    self.tick();
                }
            }
        }

        self.emit_departures();
        tracing::info!("voice receive loop exited");
    }

    fn ingest_packet(&mut self, packet: rekindle_voice::transport::VoicePacket) {
        // Skip our own packets
        if packet.sender_key == self.our_key_bytes {
            return;
        }

        self.packets_received += 1;
        let sender_key = packet.sender_key.clone();

        // Get or create participant decoder
        if !self.participants.contains_key(&sender_key) {
            match rekindle_voice::codec::OpusCodec::new(
                self.sample_rate,
                self.channels,
                self.frame_size,
            ) {
                Ok(codec) => {
                    let sender_hex = hex::encode(&sender_key);
                    tracing::info!(peer = %sender_hex, "new voice participant");

                    let event = VoiceEvent::UserJoined {
                        public_key: sender_hex.clone(),
                        display_name: sender_hex,
                    };
                    let _ = self.app.emit("voice-event", &event);

                    self.participants.insert(
                        sender_key.clone(),
                        ParticipantDecoder {
                            codec,
                            jitter_buffer: rekindle_voice::jitter::JitterBuffer::new(
                                self.jitter_buffer_ms,
                            ),
                            is_speaking: false,
                            last_packet_time: Instant::now(),
                        },
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to create decoder for participant");
                    return;
                }
            }
        }

        if let Some(participant) = self.participants.get_mut(&sender_key) {
            participant.jitter_buffer.push(packet);
            participant.last_packet_time = Instant::now();

            if !participant.is_speaking {
                participant.is_speaking = true;
                let event = VoiceEvent::UserSpeaking {
                    public_key: hex::encode(&sender_key),
                    speaking: true,
                };
                let _ = self.app.emit("voice-event", &event);
            }
        }
    }

    fn tick(&mut self) {
        let streams = self.decode_all_participants();
        if !streams.is_empty() {
            self.mix_and_send(&streams);
        }
        self.cleanup_stale_participants();
        self.update_speaking_states();
        self.log_quality_if_due();
    }

    fn decode_all_participants(&mut self) -> Vec<(String, Vec<f32>)> {
        let mut streams: Vec<(String, Vec<f32>)> = Vec::new();

        for (key, participant) in &mut self.participants {
            let decoded = match participant.jitter_buffer.pop() {
                Some(packet) => {
                    let frame = rekindle_voice::codec::EncodedFrame {
                        data: packet.audio_data,
                        timestamp: packet.timestamp,
                        sequence: packet.sequence,
                    };
                    match participant.codec.decode(&frame) {
                        Ok(decoded) => decoded.samples,
                        Err(e) => {
                            tracing::trace!(error = %e, "decode failed — using PLC");
                            participant
                                .codec
                                .decode_plc()
                                .map_or_else(|_| vec![0.0; self.frame_size], |d| d.samples)
                        }
                    }
                }
                None => {
                    // No packet available — try FEC if next packet exists, else PLC
                    if participant.last_packet_time.elapsed() < Duration::from_secs(2) {
                        if let Some(next_data) =
                            participant.jitter_buffer.peek_next_audio_data()
                        {
                            participant
                                .codec
                                .decode_fec(next_data)
                                .map_or_else(|_| vec![0.0; self.frame_size], |d| d.samples)
                        } else {
                            participant
                                .codec
                                .decode_plc()
                                .map_or_else(|_| vec![0.0; self.frame_size], |d| d.samples)
                        }
                    } else {
                        continue; // Participant timed out, skip
                    }
                }
            };

            streams.push((hex::encode(key), decoded));
        }

        streams
    }

    fn mix_and_send(&self, streams: &[(String, Vec<f32>)]) {
        let refs: Vec<(&str, &[f32])> = streams
            .iter()
            .map(|(id, samples)| (id.as_str(), samples.as_slice()))
            .collect();
        let mixed = self.mixer.mix(&refs);

        if !mixed.is_empty() {
            // Broadcast mixed audio as speaker reference for AEC
            // (before applying deafen — AEC needs what the speakers actually output)
            let _ = self.speaker_ref_tx.send(mixed.clone());

            // When deafened, send silence to keep the playback stream alive
            let output = if self.deafened_flag.load(Ordering::Relaxed) {
                vec![0.0f32; mixed.len()]
            } else {
                mixed
            };
            if self.playback_tx.try_send(output).is_err() {
                tracing::trace!("playback channel full — dropping mixed frame");
            }
        }
    }

    fn cleanup_stale_participants(&mut self) {
        let timeout_keys: Vec<Vec<u8>> = self
            .participants
            .iter()
            .filter(|(_, p)| p.last_packet_time.elapsed() > Duration::from_secs(5))
            .map(|(k, _)| k.clone())
            .collect();

        for key in timeout_keys {
            if let Some(participant) = self.participants.remove(&key) {
                let peer_hex = hex::encode(&key);
                tracing::info!(peer = %peer_hex, "voice participant timed out");

                if participant.is_speaking {
                    let event = VoiceEvent::UserSpeaking {
                        public_key: peer_hex.clone(),
                        speaking: false,
                    };
                    let _ = self.app.emit("voice-event", &event);
                }

                let event = VoiceEvent::UserLeft {
                    public_key: peer_hex,
                };
                let _ = self.app.emit("voice-event", &event);
            }
        }
    }

    fn update_speaking_states(&mut self) {
        for (key, participant) in &mut self.participants {
            if participant.is_speaking
                && participant.last_packet_time.elapsed() > Duration::from_millis(500)
            {
                participant.is_speaking = false;
                let event = VoiceEvent::UserSpeaking {
                    public_key: hex::encode(key),
                    speaking: false,
                };
                let _ = self.app.emit("voice-event", &event);
            }
        }
    }

    fn emit_departures(&self) {
        for (key, participant) in &self.participants {
            let peer_hex = hex::encode(key);
            if participant.is_speaking {
                let event = VoiceEvent::UserSpeaking {
                    public_key: peer_hex.clone(),
                    speaking: false,
                };
                let _ = self.app.emit("voice-event", &event);
            }
            let event = VoiceEvent::UserLeft {
                public_key: peer_hex,
            };
            let _ = self.app.emit("voice-event", &event);
        }
    }

    fn log_quality_if_due(&mut self) {
        if self.last_quality_check.elapsed() < Duration::from_secs(5) {
            return;
        }
        tracing::debug!(
            participants = self.participants.len(),
            self.packets_received,
            "voice receive loop stats"
        );
        self.packets_received = 0;
        self.last_quality_check = Instant::now();
    }
}
