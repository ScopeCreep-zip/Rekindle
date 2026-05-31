//! Phase 14 — MCU (Multipoint Control Unit) mixing loop for group voice.
//!
//! When this peer is elected voice host and the group has 5+ participants
//! (or any time for a stage channel — architecture §10.7 says stages
//! "always operate in relay mode"), the MCU loop receives all incoming
//! voice packets, decodes them per-sender, mixes audio for each
//! recipient (excluding their own), re-encodes, and sends via the
//! transport. Non-host participants send only to the host and receive
//! a single mixed stream back.
//!
//! **Stage audience handling (architecture §32 Phase 6 Week 20):** in
//! a stage channel only speakers transmit, so the audience never
//! appears in the per-sender decode set. The mix-out loop iterates
//! the full `transport.peer_keys()` list rather than just the
//! decoded-streams senders, so every connected peer — speaker or
//! audience — receives the speaker mix.
//!
//! No AppState/Tauri/deps coupling — moved verbatim from
//! `src-tauri/services/voice/mcu_loop.rs` (299 LoC).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::codec::{EncodedFrame, OpusCodec};
use crate::jitter::JitterBuffer;
use crate::mixer::AudioMixer;
use crate::transport::{VoicePacket, VoiceTransport};

pub struct McuParams {
    pub transport: std::sync::Arc<tokio::sync::Mutex<VoiceTransport>>,
    pub packet_rx: mpsc::Receiver<VoicePacket>,
    pub shutdown_rx: mpsc::Receiver<()>,
    pub our_key_bytes: Vec<u8>,
}

struct PerSenderState {
    codec: OpusCodec,
    jitter_buffer: JitterBuffer,
    last_packet_time: Instant,
}

struct McuLoop {
    transport: std::sync::Arc<tokio::sync::Mutex<VoiceTransport>>,
    packet_rx: mpsc::Receiver<VoicePacket>,
    shutdown_rx: mpsc::Receiver<()>,
    our_key_bytes: Vec<u8>,
    senders: HashMap<Vec<u8>, PerSenderState>,
    mixer: AudioMixer,
    encoder: OpusCodec,
    frame_size: usize,
    sequence: u32,
}

pub async fn run(params: McuParams) {
    let Some(loop_state) = McuLoop::new(params) else {
        return;
    };
    loop_state.run_loop().await;
}

impl McuLoop {
    fn new(params: McuParams) -> Option<Self> {
        let sample_rate: u32 = 48000;
        let channels: u16 = 1;
        let frame_size: usize = 960;

        let encoder = match OpusCodec::new(sample_rate, channels, frame_size) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "MCU loop: failed to create encoder");
                return None;
            }
        };

        Some(Self {
            transport: params.transport,
            packet_rx: params.packet_rx,
            shutdown_rx: params.shutdown_rx,
            our_key_bytes: params.our_key_bytes,
            senders: HashMap::new(),
            mixer: AudioMixer::new(channels),
            encoder,
            frame_size,
            sequence: 0,
        })
    }

    async fn run_loop(mut self) {
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tracing::info!("MCU mix loop started");

        loop {
            tokio::select! {
                biased;
                _ = self.shutdown_rx.recv() => {
                    tracing::info!("MCU mix loop: shutdown signal received");
                    break;
                }
                Some(packet) = self.packet_rx.recv() => {
                    self.ingest_packet(packet);
                }
                _ = tick.tick() => {
                    self.tick().await;
                }
            }
        }
        tracing::info!("MCU mix loop exited");
    }

    fn ingest_packet(&mut self, packet: VoicePacket) {
        if packet.sender_key == self.our_key_bytes {
            return;
        }
        let sender_key = packet.sender_key.clone();
        if !self.senders.contains_key(&sender_key) {
            match OpusCodec::new(48000, 1, self.frame_size) {
                Ok(codec) => {
                    self.senders.insert(
                        sender_key.clone(),
                        PerSenderState {
                            codec,
                            jitter_buffer: JitterBuffer::new(200),
                            last_packet_time: Instant::now(),
                        },
                    );
                }
                Err(e) => {
                    tracing::warn!(error = %e, "MCU: failed to create decoder for sender");
                    return;
                }
            }
        }
        if let Some(sender) = self.senders.get_mut(&sender_key) {
            sender.jitter_buffer.push(packet);
            sender.last_packet_time = Instant::now();
        }
    }

    async fn tick(&mut self) {
        // 1. Decode one frame from each sender.
        let mut decoded_streams: Vec<(Vec<u8>, Vec<f32>)> = Vec::new();
        for (key, sender) in &mut self.senders {
            if let Some(packet) = sender.jitter_buffer.pop() {
                let frame = EncodedFrame {
                    data: packet.audio_data,
                    timestamp: packet.timestamp,
                    sequence: packet.sequence,
                };
                match sender.codec.decode(&frame) {
                    Ok(decoded) => {
                        decoded_streams.push((key.clone(), decoded.samples));
                    }
                    Err(_) => {
                        if let Ok(plc) = sender.codec.decode_plc() {
                            decoded_streams.push((key.clone(), plc.samples));
                        }
                    }
                }
            }
        }

        if decoded_streams.is_empty() {
            self.cleanup_stale_senders();
            return;
        }

        // 2. Build the recipient set from the FULL connected peer list.
        let recipient_keys: Vec<Vec<u8>> = {
            let transport = self.transport.lock().await;
            select_recipients(&transport.peer_keys(), &self.our_key_bytes)
        };
        let hex_keys: Vec<String> = decoded_streams.iter().map(|(key, _)| hex::encode(key)).collect();

        for recipient_key in &recipient_keys {
            let streams_for_recipient: Vec<(&str, &[f32])> = decoded_streams
                .iter()
                .zip(hex_keys.iter())
                .filter(|((key, _), _)| key != recipient_key)
                .map(|((_, samples), hex_key)| (hex_key.as_str(), samples.as_slice()))
                .collect();

            if streams_for_recipient.is_empty() {
                continue;
            }

            let mixed = self.mixer.mix(&streams_for_recipient);

            match self.encoder.encode(&mixed) {
                Ok(mut encoded) => {
                    encoded.sequence = self.sequence;
                    encoded.timestamp = rekindle_utils::timestamp_ms();
                    self.sequence = self.sequence.wrapping_add(1);

                    let recipient_hex = hex::encode(recipient_key);
                    let transport = self.transport.clone();
                    tokio::spawn(async move {
                        let t = transport.lock().await;
                        if let Err(e) = t.send_to_peer(&recipient_hex, &encoded).await {
                            tracing::trace!(error = %e, peer = %recipient_hex, "MCU send failed");
                        }
                    });
                }
                Err(e) => {
                    tracing::trace!(error = %e, "MCU: encode failed for mixed stream");
                }
            }
        }

        self.cleanup_stale_senders();
    }

    fn cleanup_stale_senders(&mut self) {
        let stale: Vec<Vec<u8>> = self
            .senders
            .iter()
            .filter(|(_, s)| s.last_packet_time.elapsed() > Duration::from_secs(5))
            .map(|(k, _)| k.clone())
            .collect();

        for key in stale {
            self.senders.remove(&key);
            tracing::debug!(sender = %hex::encode(&key), "MCU: removed stale sender");
        }
    }
}

/// Pure helper extracted from `tick`: given the connected peer list
/// and our own pseudonym, return the set of recipients the host
/// should mix audio for. Excludes self; includes audience members
/// (peers who didn't send a packet this tick).
fn select_recipients(peer_keys_hex: &[String], self_key_bytes: &[u8]) -> Vec<Vec<u8>> {
    peer_keys_hex
        .iter()
        .filter_map(|hex_key| hex::decode(hex_key).ok())
        .filter(|raw| raw != self_key_bytes)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::select_recipients;

    #[test]
    fn select_recipients_excludes_self() {
        let me = vec![0u8; 32];
        let me_hex = hex::encode(&me);
        let peer = vec![1u8; 32];
        let peer_hex = hex::encode(&peer);
        let recipients = select_recipients(&[me_hex, peer_hex], &me);
        assert_eq!(recipients, vec![peer]);
    }

    #[test]
    fn select_recipients_includes_silent_audience() {
        let alice = vec![1u8; 32];
        let bob = vec![2u8; 32];
        let carol = vec![3u8; 32];
        let dave = vec![4u8; 32];
        let peers = vec![
            hex::encode(&alice),
            hex::encode(&bob),
            hex::encode(&carol),
            hex::encode(&dave),
        ];
        let recipients = select_recipients(&peers, &alice);
        assert_eq!(recipients.len(), 3);
        assert!(recipients.contains(&bob));
        assert!(recipients.contains(&carol));
        assert!(recipients.contains(&dave));
    }

    #[test]
    fn select_recipients_drops_invalid_hex() {
        let me = vec![0u8; 32];
        let recipients =
            select_recipients(&["not-hex".to_string(), hex::encode(&me)], &me);
        assert!(recipients.is_empty(), "self filtered + invalid dropped");
    }
}
