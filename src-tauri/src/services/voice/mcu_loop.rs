//! MCU (Multipoint Control Unit) mixing loop for group voice.
//!
//! When this peer is elected voice host and the group has 6+ participants,
//! the MCU loop receives all incoming voice packets, decodes them per-sender,
//! mixes audio for each recipient (excluding their own), re-encodes, and sends
//! via the transport. Non-host participants send only to the host and receive
//! a single mixed stream back.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

pub(crate) struct McuParams {
    pub transport: std::sync::Arc<tokio::sync::Mutex<rekindle_voice::transport::VoiceTransport>>,
    pub packet_rx: mpsc::Receiver<rekindle_voice::transport::VoicePacket>,
    pub shutdown_rx: mpsc::Receiver<()>,
    pub our_key_bytes: Vec<u8>,
}

struct PerSenderState {
    codec: rekindle_voice::codec::OpusCodec,
    jitter_buffer: rekindle_voice::jitter::JitterBuffer,
    last_packet_time: Instant,
}

struct McuLoop {
    transport: std::sync::Arc<tokio::sync::Mutex<rekindle_voice::transport::VoiceTransport>>,
    packet_rx: mpsc::Receiver<rekindle_voice::transport::VoicePacket>,
    shutdown_rx: mpsc::Receiver<()>,
    our_key_bytes: Vec<u8>,
    senders: HashMap<Vec<u8>, PerSenderState>,
    mixer: rekindle_voice::mixer::AudioMixer,
    encoder: rekindle_voice::codec::OpusCodec,
    frame_size: usize,
    sequence: u32,
}

pub(crate) async fn run(params: McuParams) {
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

        let encoder = match rekindle_voice::codec::OpusCodec::new(sample_rate, channels, frame_size)
        {
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
            mixer: rekindle_voice::mixer::AudioMixer::new(channels),
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
                    self.tick();
                }
            }
        }

        tracing::info!("MCU mix loop exited");
    }

    fn ingest_packet(&mut self, packet: rekindle_voice::transport::VoicePacket) {
        // Skip our own packets
        if packet.sender_key == self.our_key_bytes {
            return;
        }

        let sender_key = packet.sender_key.clone();

        // Get or create per-sender state
        if !self.senders.contains_key(&sender_key) {
            match rekindle_voice::codec::OpusCodec::new(48000, 1, self.frame_size) {
                Ok(codec) => {
                    self.senders.insert(
                        sender_key.clone(),
                        PerSenderState {
                            codec,
                            jitter_buffer: rekindle_voice::jitter::JitterBuffer::new(200),
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

    fn tick(&mut self) {
        // 1. Decode one frame from each sender
        let mut decoded_streams: Vec<(Vec<u8>, Vec<f32>)> = Vec::new();
        for (key, sender) in &mut self.senders {
            if let Some(packet) = sender.jitter_buffer.pop() {
                let frame = rekindle_voice::codec::EncodedFrame {
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

        // 2. For each recipient: mix all EXCEPT their own audio, encode, send
        let all_keys: Vec<Vec<u8>> = decoded_streams.iter().map(|(k, _)| k.clone()).collect();

        for recipient_key in &all_keys {
            let hex_keys: Vec<String> = decoded_streams
                .iter()
                .map(|(key, _)| hex::encode(key))
                .collect();
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

                    // Find the recipient's pseudonym key to send to them
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
