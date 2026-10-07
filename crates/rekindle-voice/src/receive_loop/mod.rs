//! Phase 14 — voice packet receive → decode → mix → playback pipeline.
//!
//! Receives `VoicePacket`s from the dispatch loop, decodes per-participant
//! using Opus (with FEC/PLC fallback), mixes with `AudioMixer`, and
//! pushes to the playback channel on a 20 ms tick.
//!
//! Parameterized over `Arc<dyn VoiceSessionDeps>` for AppState lookups
//! (MEK, active calls, stage gate, member-name resolution).
//! Pre-Phase-14 this lived in `src-tauri/services/voice/receive_loop.rs`
//! (463 LoC).

mod quality;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{broadcast, mpsc};

use crate::codec::{EncodedFrame, OpusCodec};
use crate::jitter::{JitterBuffer, JitterFrame};
use crate::liveness::MediaLiveness;
use crate::media_crypto::{FrameOpener, MediaKeys, MediaScope, OpenError};
use crate::mixer::AudioMixer;
use crate::receiver_report::SenderEcho;
use crate::session_deps::{MediaKeySource, VoiceSessionDeps, VoiceSessionEvent};
use crate::transport::VoicePacket;

pub struct VoiceReceiveParams {
    pub packet_rx: mpsc::Receiver<VoicePacket>,
    pub playback_tx: Option<mpsc::Sender<Vec<f32>>>,
    /// Cancelled when the loop's session scope shuts down.
    pub stop: tokio_util::sync::CancellationToken,
    pub deps: Arc<dyn VoiceSessionDeps>,
    pub our_public_key: String,
    pub deafened_flag: Arc<AtomicBool>,
    pub speaker_ref_tx: broadcast::Sender<Vec<f32>>,
    /// Community ID for MEK decryption. `None` for 1:1 calls.
    pub community_id: Option<String>,
    /// Channel ID for stage-gate lookup. `None` for 1:1 calls.
    pub channel_id: Option<String>,
    /// Pre-loaded member display names: `pseudonym_key_hex → display_name`.
    /// Populated from SQLite on loop start so we don't need async DB
    /// queries inside the receive path.
    pub member_names: HashMap<String, String>,
    /// Base jitter target (ms) from `VoiceConfig` — the floor the
    /// adaptive controller starts at and never shrinks below. Replaces
    /// the old hardcoded 200 ms.
    pub jitter_base_ms: u32,
    /// Identity that signs our receiver reports — the community
    /// pseudonym key for channel voice, the account key for a 1:1 call,
    /// matching whatever `our_public_key` names and whatever the send
    /// side signs packets with. `None` disables reporting rather than
    /// sending reports a peer would reject.
    pub report_signing_key: Option<ed25519_dalek::SigningKey>,
    /// Session-shared media-plane liveness ledger. Every accepted
    /// packet notes the sender so the presence reconcile never evicts
    /// (or fails to add) a peer whose audio is actually flowing —
    /// their DHT presence row goes stale exactly when the call
    /// saturates the relays their presence writes need.
    pub media_liveness: Arc<MediaLiveness>,
}

struct ParticipantDecoder {
    codec: OpusCodec,
    jitter_buffer: JitterBuffer,
    /// Newest accepted packet from this peer, for the RFC 3550 LSR/DLSR
    /// echo that lets *them* compute the round trip.
    echo: SenderEcho,
    is_speaking: bool,
    last_packet_time: Instant,
}

struct VoiceReceiveLoop {
    packet_rx: mpsc::Receiver<VoicePacket>,
    playback_tx: mpsc::Sender<Vec<f32>>,
    stop: tokio_util::sync::CancellationToken,
    deps: Arc<dyn VoiceSessionDeps>,
    our_key_bytes: Vec<u8>,
    deafened_flag: Arc<AtomicBool>,
    speaker_ref_tx: broadcast::Sender<Vec<f32>>,
    participants: HashMap<Vec<u8>, ParticipantDecoder>,
    mixer: AudioMixer,
    frame_size: usize,
    sample_rate: u32,
    channels: u16,
    jitter_base_ms: u32,
    packets_received: u64,
    last_quality_check: Instant,
    /// Packets dropped this stats window because they could not be
    /// opened (no key from their sender yet, wrong sender, or rejected).
    key_drops: u64,
    /// SFrame-opens inbound frames; owns the per-sender CTR replay
    /// windows (M9.3), so replays are dropped before the jitter buffer.
    opener: FrameOpener,
    /// When we last asked each sender for a key: one request per window
    /// even when every packet of a 50/s stream is undecryptable.
    last_key_request: HashMap<String, Instant>,
    community_id: Option<String>,
    channel_id: Option<String>,
    member_names: HashMap<String, String>,
    report_signing_key: Option<ed25519_dalek::SigningKey>,
    media_liveness: Arc<MediaLiveness>,
    /// Origin for the loop's local millisecond clock. Only differences
    /// within it are ever used (DLSR is a duration), so the origin is
    /// arbitrary — it just has to be monotonic and stable for the
    /// session.
    origin: Instant,
}

/// Entry point: validate params, build loop state, run until shutdown.
pub async fn run(params: VoiceReceiveParams) {
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

        let key_source: Arc<dyn MediaKeySource> = params.deps.clone();
        let keys = Arc::new(MediaKeys::new(
            key_source,
            MediaScope::of_session(
                params.community_id.as_deref(),
                params.channel_id.as_deref().unwrap_or_default(),
            ),
        ));
        Some(Self {
            opener: FrameOpener::new(keys),
            packet_rx: params.packet_rx,
            playback_tx,
            stop: params.stop,
            deps: params.deps,
            our_key_bytes: hex::decode(&params.our_public_key).unwrap_or_default(),
            deafened_flag: params.deafened_flag,
            speaker_ref_tx: params.speaker_ref_tx,
            participants: HashMap::new(),
            mixer: AudioMixer::new(channels),
            frame_size,
            sample_rate,
            channels,
            jitter_base_ms: params.jitter_base_ms,
            packets_received: 0,
            last_quality_check: Instant::now(),
            key_drops: 0,
            last_key_request: HashMap::new(),
            community_id: params.community_id,
            channel_id: params.channel_id,
            member_names: params.member_names,
            report_signing_key: params.report_signing_key,
            media_liveness: params.media_liveness,
            origin: Instant::now(),
        })
    }

    /// Local milliseconds since loop start — the clock the LSR/DLSR
    /// echo measures its own delay in.
    fn local_ms(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Look up a member's display name from the pre-loaded cache.
    /// Falls back to the hex-encoded pseudonym key if not found.
    fn resolve_display_name(&self, pseudonym_hex: &str) -> String {
        self.member_names
            .get(pseudonym_hex)
            .cloned()
            .unwrap_or_else(|| pseudonym_hex.to_string())
    }

    async fn run_loop(mut self) {
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tracing::info!("voice receive loop started");

        loop {
            tokio::select! {
                biased;
                () = self.stop.cancelled() => {
                    tracing::info!("voice receive loop: shutdown signal received");
                    break;
                }
                Some(packet) = self.packet_rx.recv() => {
                    self.ingest_packet(&packet);
                }
                _ = tick.tick() => {
                    self.tick();
                }
            }
        }

        self.emit_departures();
        tracing::info!("voice receive loop exited");
    }

    fn ingest_packet(&mut self, packet: &VoicePacket) {
        // Skip our own packets.
        if packet.sender_key == self.our_key_bytes {
            return;
        }

        if !self.sender_allowed_for_stage(&packet.sender_key) {
            return;
        }

        // SFrame open under the sender's key (RFC 9605). Failures are
        // counted, not logged per packet (§4.4.4); a key the sender has not
        // sent us is requested from that sender, debounced (plan C7.20).
        let opus = match self.opener.open(packet) {
            Ok(opus) => opus,
            Err(OpenError::NoKey { needed_index }) => {
                if let (Some(cid), Some(needed)) = (self.community_id.clone(), needed_index) {
                    let channel = self.channel_id.clone().unwrap_or_default();
                    let sender = hex::encode(&packet.sender_key);
                    self.note_missing_key(&cid, &channel, &sender, needed);
                } else {
                    self.key_drops += 1;
                    self.deps.record_packet_drop();
                }
                return;
            }
            Err(OpenError::WrongSender | OpenError::Rejected) => {
                self.key_drops += 1;
                self.deps.record_packet_drop();
                return;
            }
        };

        self.packets_received += 1;
        let sender_key = packet.sender_key.clone();

        // Get or create participant decoder.
        if !self.participants.contains_key(&sender_key) {
            match OpusCodec::new(self.sample_rate, self.channels, self.frame_size) {
                Ok(codec) => {
                    let sender_hex = hex::encode(&sender_key);
                    tracing::info!(peer = %sender_hex, "new voice participant");

                    let display_name = self.resolve_display_name(&sender_hex);
                    self.deps.emit_voice_event(VoiceSessionEvent::UserJoined {
                        peer_pubkey: sender_hex,
                        display_name,
                    });

                    self.participants.insert(
                        sender_key.clone(),
                        ParticipantDecoder {
                            codec,
                            echo: SenderEcho::default(),
                            jitter_buffer: JitterBuffer::new(self.jitter_base_ms),
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

        let arrival_local_ms = self.local_ms();
        if let Some(participant) = self.participants.get_mut(&sender_key) {
            // Remember the newest accepted packet so our next report
            // lets this peer compute the round trip.
            participant.echo.observe(packet, arrival_local_ms);
            participant.jitter_buffer.push(
                JitterFrame {
                    sequence: packet.sequence,
                    timestamp: packet.timestamp,
                    opus,
                },
                arrival_local_ms,
            );
            participant.last_packet_time = Instant::now();
            // Media-plane proof of life for the presence reconcile.
            // Wall-clock ms (`timestamp_ms`), NOT `local_ms`: the
            // ledger is shared with the send loop's receiver-report
            // path, and the loops' `Instant` origins differ — only the
            // wall clock is a domain both sides already stamp.
            self.media_liveness
                .note(&hex::encode(&sender_key), rekindle_utils::timestamp_ms());

            if !participant.is_speaking {
                participant.is_speaking = true;
                self.deps.emit_voice_event(VoiceSessionEvent::UserSpeaking {
                    peer_pubkey: hex::encode(&sender_key),
                    speaking: true,
                });
            }
        }
    }

    fn sender_allowed_for_stage(&self, sender_key: &[u8]) -> bool {
        let (Some(ref community_id), Some(ref channel_id)) = (&self.community_id, &self.channel_id)
        else {
            return true;
        };
        if !self.deps.channel_is_stage(community_id, channel_id) {
            return true;
        }
        let sender_hex = hex::encode(sender_key);
        self.deps
            .sender_is_stage_speaker(community_id, channel_id, &sender_hex)
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

        // One playout-tick clock reading, shared by every participant's
        // jitter buffer this tick — the same monotonic ms clock `push`
        // stamps arrivals with, so the initial-fill window measures real
        // elapsed time.
        let now_ms = self.local_ms();
        let frame_size = self.frame_size;
        let decode_packet = |participant: &mut ParticipantDecoder, packet: JitterFrame| {
            let frame = EncodedFrame {
                data: packet.opus,
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
                        .map_or_else(|_| vec![0.0; frame_size], |d| d.samples)
                }
            }
        };

        for (key, participant) in &mut self.participants {
            let decoded = match participant.jitter_buffer.pop(now_ms) {
                Some(packet) => decode_packet(participant, packet),
                None => {
                    if participant.last_packet_time.elapsed() < Duration::from_secs(2) {
                        // Expected packet missing. Once the gap outlives the
                        // jitter window it is loss, not reordering — jump and
                        // resume from the oldest buffered packet (a stalled
                        // position otherwise discards every later GOOD packet
                        // via the overflow trim). Until then: FEC off the next
                        // packet when buffered (advancing past the concealed
                        // position), else PLC.
                        if let Some(packet) = participant.jitter_buffer.note_miss_and_maybe_jump() {
                            decode_packet(participant, packet)
                        } else if let Some(next_data) =
                            participant.jitter_buffer.peek_next_audio_data()
                        {
                            let samples = participant
                                .codec
                                .decode_fec(next_data)
                                .map_or_else(|_| vec![0.0; frame_size], |d| d.samples);
                            participant.jitter_buffer.advance_after_fec();
                            samples
                        } else {
                            participant
                                .codec
                                .decode_plc()
                                .map_or_else(|_| vec![0.0; frame_size], |d| d.samples)
                        }
                    } else {
                        continue; // Participant timed out, skip.
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
            // (before applying deafen — AEC needs what speakers
            // actually output).
            let _ = self.speaker_ref_tx.send(mixed.clone());

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
                    self.deps.emit_voice_event(VoiceSessionEvent::UserSpeaking {
                        peer_pubkey: peer_hex.clone(),
                        speaking: false,
                    });
                }
                self.deps.emit_voice_event(VoiceSessionEvent::UserLeft {
                    peer_pubkey: peer_hex,
                });
            }
        }
    }

    fn update_speaking_states(&mut self) {
        for (key, participant) in &mut self.participants {
            if participant.is_speaking
                && participant.last_packet_time.elapsed() > Duration::from_millis(500)
            {
                participant.is_speaking = false;
                self.deps.emit_voice_event(VoiceSessionEvent::UserSpeaking {
                    peer_pubkey: hex::encode(key),
                    speaking: false,
                });
            }
        }
    }

    fn emit_departures(&self) {
        for (key, participant) in &self.participants {
            let peer_hex = hex::encode(key);
            if participant.is_speaking {
                self.deps.emit_voice_event(VoiceSessionEvent::UserSpeaking {
                    peer_pubkey: peer_hex.clone(),
                    speaking: false,
                });
            }
            self.deps.emit_voice_event(VoiceSessionEvent::UserLeft {
                peer_pubkey: peer_hex,
            });
        }
    }
}
