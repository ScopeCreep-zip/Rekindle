use crate::error::VoiceError;

/// Opus codec wrapper for encoding and decoding voice frames.
///
/// Wraps `opus::Encoder` and `opus::Decoder` configured for `VoIP`-optimised
/// speech at the given sample rate and channel count. Frame size must match
/// one of the Opus-valid durations (2.5, 5, 10, 20, 40, or 60 ms).
pub struct OpusCodec {
    encoder: opus::Encoder,
    decoder: opus::Decoder,
    sample_rate: u32,
    channels: u16,
    frame_size: usize,
}

/// A single encoded audio frame.
pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub timestamp: u64,
    pub sequence: u32,
}

/// A single decoded audio frame (PCM samples).
pub struct DecodedFrame {
    pub samples: Vec<f32>,
    pub timestamp: u64,
}

fn to_opus_channels(channels: u16) -> Result<opus::Channels, VoiceError> {
    match channels {
        1 => Ok(opus::Channels::Mono),
        2 => Ok(opus::Channels::Stereo),
        n => Err(VoiceError::Codec(format!(
            "unsupported channel count: {n} (only mono/stereo)"
        ))),
    }
}

impl OpusCodec {
    /// Create a new Opus codec with the given parameters.
    ///
    /// The encoder is configured for `VoIP` application mode which prioritises
    /// low-latency speech. Valid `frame_size` values at 48 kHz are:
    /// 120, 240, 480, 960, 1920, 2880 (corresponding to 2.5–60 ms).
    pub fn new(sample_rate: u32, channels: u16, frame_size: usize) -> Result<Self, VoiceError> {
        let opus_channels = to_opus_channels(channels)?;

        let mut encoder = opus::Encoder::new(sample_rate, opus_channels, opus::Application::Voip)
            .map_err(|e| VoiceError::Codec(format!("encoder init failed: {e}")))?;

        // Configure encoder for voice over Veilid:
        // - 32kbps bitrate: good quality speech, reduces P2P relay load
        // - In-band FEC: allows partial recovery from packet loss
        // - 10% expected packet loss: tells Opus to include enough FEC data
        encoder
            .set_bitrate(opus::Bitrate::Bits(32000))
            .map_err(|e| VoiceError::Codec(format!("set bitrate failed: {e}")))?;
        encoder
            .set_inband_fec(true)
            .map_err(|e| VoiceError::Codec(format!("set FEC failed: {e}")))?;
        encoder
            .set_packet_loss_perc(10)
            .map_err(|e| VoiceError::Codec(format!("set packet loss percent failed: {e}")))?;

        let decoder = opus::Decoder::new(sample_rate, opus_channels)
            .map_err(|e| VoiceError::Codec(format!("decoder init failed: {e}")))?;

        Ok(Self {
            encoder,
            decoder,
            sample_rate,
            channels,
            frame_size,
        })
    }

    /// Encode PCM f32 samples to an Opus frame.
    ///
    /// `pcm` must contain exactly `frame_size * channels` samples.
    /// Returns the encoded frame with the caller-provided timestamp/sequence
    /// set to zero (the caller is expected to fill these in).
    pub fn encode(&mut self, pcm: &[f32]) -> Result<EncodedFrame, VoiceError> {
        // Opus can produce at most 1275 bytes per frame for standard modes,
        // but 4000 is the recommended safe ceiling from the docs.
        let mut output = vec![0u8; 4000];
        let len = self
            .encoder
            .encode_float(pcm, &mut output)
            .map_err(|e| VoiceError::Codec(format!("encode failed: {e}")))?;
        output.truncate(len);

        Ok(EncodedFrame {
            data: output,
            timestamp: 0,
            sequence: 0,
        })
    }

    /// Decode an Opus frame back to PCM f32 samples.
    pub fn decode(&mut self, frame: &EncodedFrame) -> Result<DecodedFrame, VoiceError> {
        let channels_usize = usize::from(self.channels);
        let max_samples = self.frame_size * channels_usize;
        let mut output = vec![0.0f32; max_samples];
        let decoded_samples = self
            .decoder
            .decode_float(&frame.data, &mut output, false)
            .map_err(|e| VoiceError::Codec(format!("decode failed: {e}")))?;
        // decode_float returns samples *per channel*
        output.truncate(decoded_samples * channels_usize);

        Ok(DecodedFrame {
            samples: output,
            timestamp: frame.timestamp,
        })
    }

    /// Decode using forward error correction data from a subsequent packet.
    ///
    /// When a packet is missing but the *next* packet has arrived, call this
    /// with the next packet's data to recover the missing frame using Opus FEC.
    pub fn decode_fec(&mut self, next_packet_data: &[u8]) -> Result<DecodedFrame, VoiceError> {
        let channels_usize = usize::from(self.channels);
        let max_samples = self.frame_size * channels_usize;
        let mut output = vec![0.0f32; max_samples];
        let decoded_samples = self
            .decoder
            .decode_float(next_packet_data, &mut output, true) // fec=true
            .map_err(|e| VoiceError::Codec(format!("FEC decode failed: {e}")))?;
        output.truncate(decoded_samples * channels_usize);

        Ok(DecodedFrame {
            samples: output,
            timestamp: 0,
        })
    }

    /// Perform packet-loss concealment (decode with no input).
    ///
    /// Generates a frame that smoothly fills the gap left by a missing packet.
    pub fn decode_plc(&mut self) -> Result<DecodedFrame, VoiceError> {
        let channels_usize = usize::from(self.channels);
        let max_samples = self.frame_size * channels_usize;
        let mut output = vec![0.0f32; max_samples];
        let decoded_samples = self
            .decoder
            .decode_float(&[], &mut output, false)
            .map_err(|e| VoiceError::Codec(format!("PLC decode failed: {e}")))?;
        output.truncate(decoded_samples * channels_usize);

        Ok(DecodedFrame {
            samples: output,
            timestamp: 0,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn frame_size(&self) -> usize {
        self.frame_size
    }

    /// Update the expected packet loss percentage for the encoder.
    ///
    /// Affects how much FEC data Opus includes. Higher values = more redundancy
    /// but larger packets.
    pub fn set_packet_loss_perc(&mut self, percent: i32) -> Result<(), VoiceError> {
        self.encoder
            .set_packet_loss_perc(percent)
            .map_err(|e| VoiceError::Codec(format!("set packet loss percent failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let frame_size = 960; // 20ms at 48kHz
        let mut codec = OpusCodec::new(48000, 1, frame_size).unwrap();

        // Generate a simple sine wave
        let pcm: Vec<f32> = (0..frame_size)
            .map(|i: usize| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32;
                (t * 440.0 * std::f32::consts::TAU / 48000.0).sin() * 0.5
            })
            .collect();

        let encoded = codec.encode(&pcm).unwrap();
        // Opus should compress speech-like signals significantly
        assert!(encoded.data.len() < pcm.len() * 4);

        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded.samples.len(), frame_size);
    }

    #[test]
    fn test_silence_encodes_small() {
        let frame_size = 960;
        let mut codec = OpusCodec::new(48000, 1, frame_size).unwrap();

        let silence = vec![0.0f32; frame_size];
        let encoded = codec.encode(&silence).unwrap();
        // Opus encodes silence much smaller than raw PCM (3840 bytes)
        assert!(
            encoded.data.len() < 200,
            "expected small encoded size, got {}",
            encoded.data.len()
        );
    }
}
