use crate::error::VoiceError;

/// Starting Opus bitrate, matching the per-channel default the major
/// voice services ship. The send loop's group ladder scales down from
/// here as the mesh grows and a struggling link backs off further, but
/// never below [`MIN_BITRATE_BPS`].
pub const DEFAULT_BITRATE_BPS: i32 = 64_000;

/// Floor for every bitrate decision.
///
/// Kept well above libopus's "very low bit-rate" region because we run
/// hard CBR (see [`OpusCodec::new`]): that is where the docs warn CBR
/// costs noticeable quality, and CBR is a security setting we will not
/// trade away to save bandwidth.
pub const MIN_BITRATE_BPS: i32 = 24_000;

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
    /// Generation of the channel-media MEK that encrypted `data`
    /// (0 = unencrypted / 1:1 call). Stamped by the send loop at
    /// encrypt time; carried into the wire packet so receivers can
    /// detect generation mismatch instead of decrypt-failing blind.
    pub mek_generation: u64,
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

        // Configure encoder for anonymous voice over Veilid.
        //
        // 64 kbps matches the default other major services use for a
        // voice channel; the send loop's group ladder scales it down
        // from here as the mesh grows.
        encoder
            .set_bitrate(opus::Bitrate::Bits(DEFAULT_BITRATE_BPS))
            .map_err(|e| VoiceError::Codec(format!("set bitrate failed: {e}")))?;

        // Hard CBR — a SECURITY setting, not a quality one.
        //
        // Opus defaults to VBR, where packet size tracks the phoneme
        // being encoded. That is a published attack on encrypted VoIP:
        // an observer who never breaks the encryption can recover
        // phrases from the packet-length sequence alone. RFC 6562
        // ("Guidelines for the Use of VBR Audio with Secure RTP") is
        // explicit: applications "conveying highly sensitive
        // unstructured information SHOULD NOT use codecs in VBR mode",
        // and a CBR codec "SHOULD be negotiated and used instead, or
        // the VBR codec SHOULD be operated in a CBR mode".
        //
        // We are that application by construction. This project spends
        // ~75 ms of mouth-to-ear budget on a 3-hop anonymising route
        // and refuses `SafetySelection::Unsafe` on every frame; leaking
        // the words through packet sizes afterwards would make that
        // payment pointless. Constant size carries zero information.
        //
        // Note the coupling to the bitrate above: libopus warns hard
        // CBR "can cause noticeable quality degradation" for LPC/hybrid
        // modes *at very low bit-rate*. Raising the rate is what makes
        // CBR cheap here, which is why the two land together and why
        // the ladder's floor is kept well clear of that region.
        encoder
            .set_vbr(false)
            .map_err(|e| VoiceError::Codec(format!("set CBR failed: {e}")))?;

        // In-band FEC lets the decoder reconstruct a lost frame from
        // the next one. Under hard CBR the redundancy is taken out of
        // the frame's fixed budget rather than added to it, so enabling
        // it costs quality, never packet size — and so cannot reopen
        // the size channel CBR just closed. The send loop retunes
        // `packet_loss_perc` from what the far end actually reports;
        // 10 % is only the pre-first-report starting point.
        encoder
            .set_inband_fec(true)
            .map_err(|e| VoiceError::Codec(format!("set FEC failed: {e}")))?;
        encoder
            .set_packet_loss_perc(10)
            .map_err(|e| VoiceError::Codec(format!("set packet loss percent failed: {e}")))?;

        // DTX is deliberately left off (libopus defaults it off). It
        // suppresses transmission during silence, which is the same
        // talk-pattern leak RFC 6562 warns about under VAD — and we
        // already gate on VAD in the send loop, so enabling DTX would
        // add nothing but a second copy of that exposure.

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
            mek_generation: 0,
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

    /// Update the encoder bitrate for adaptive quality based on group size.
    ///
    /// Recommended values:
    /// - 2-3 participants: 32000 bps (default)
    /// - 4-8 participants: 24000 bps
    /// - 9+  participants: 16000 bps
    pub fn set_bitrate(&mut self, bps: i32) -> Result<(), VoiceError> {
        self.encoder
            .set_bitrate(opus::Bitrate::Bits(bps))
            .map_err(|e| VoiceError::Codec(format!("set bitrate failed: {e}")))
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

    /// Hard CBR is a security property, so it is pinned behaviourally
    /// rather than by asserting we called a setter.
    ///
    /// Under VBR, Opus frame length tracks the phoneme being encoded —
    /// the published attack on encrypted VoIP recovers phrases from the
    /// length sequence without touching the ciphertext. RFC 6562 says
    /// such applications SHOULD run the codec in a CBR mode. This feeds
    /// the encoder wildly different signals (silence, a tone, and
    /// noise) and asserts the packets come out the same size, which is
    /// what "carries zero information" actually means.
    ///
    /// A regression here is silent and invisible in a call: audio still
    /// works perfectly while leaking the words.
    #[test]
    fn cbr_makes_packet_size_independent_of_content() {
        let frame_size = 960; // 20 ms at 48 kHz
        let mut codec = OpusCodec::new(48_000, 1, frame_size).expect("codec");

        let silence = vec![0.0f32; frame_size];
        let tone: Vec<f32> = (0..frame_size)
            .map(|i| {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "frame_size is 960; f32 is exact well past that"
                )]
                let t = i as f32 / 48_000.0;
                (t * 440.0 * std::f32::consts::TAU).sin() * 0.5
            })
            .collect();
        // Deterministic pseudo-noise — the hardest case to encode, and
        // the one VBR would spend the most bits on.
        let noise: Vec<f32> = (0..frame_size)
            .map(|i| {
                let x = u32::try_from(i).unwrap_or(0).wrapping_mul(2_654_435_761);
                let hi = u16::try_from(x >> 16).unwrap_or(0);
                (f32::from(hi) / 32_768.0) - 1.0
            })
            .collect();

        // Warm up: Opus adapts over the first frames, so compare only
        // once the encoder has settled into steady state.
        for _ in 0..10 {
            codec.encode(&tone).expect("warmup");
        }

        let mut sizes = Vec::new();
        for pcm in [&silence, &tone, &noise, &silence, &noise] {
            sizes.push(codec.encode(pcm).expect("encode").data.len());
        }

        let min = sizes.iter().min().copied().unwrap_or(0);
        let max = sizes.iter().max().copied().unwrap_or(0);
        // libopus rounds each frame to a whole number of bytes, so hard
        // CBR is constant to within a byte or two, not bit-exact.
        assert!(
            max - min <= 2,
            "packet size varied with content ({min}..{max} bytes): VBR is on and the \
             encrypted stream leaks phonemes"
        );
    }

    /// The ladder must never drive the encoder into the region where
    /// libopus documents CBR as costing noticeable quality.
    ///
    /// `const` blocks, so lowering a constant fails the **build** rather
    /// than a test run — this guards a security setting, and the whole
    /// point is that it cannot be relaxed quietly.
    #[test]
    fn the_bitrate_floor_stays_clear_of_the_low_rate_cbr_penalty() {
        const { assert!(MIN_BITRATE_BPS >= 24_000) };
        const { assert!(DEFAULT_BITRATE_BPS >= MIN_BITRATE_BPS) };
        // Pin the worst bitrate the system can ever emit: the smallest
        // group rung (8+ peers) with a Poor-link backoff on top. The
        // ladder's own arithmetic lands under the floor there, so the
        // clamp in `report_quality_if_due` is load-bearing rather than
        // decorative — worth stating, because deleting it as redundant
        // would silently reopen the low-rate CBR quality hole.
        const {
            let worst_rung = DEFAULT_BITRATE_BPS / 2;
            assert!(
                worst_rung * 2 / 3 < MIN_BITRATE_BPS,
                "clamp is load-bearing"
            );
        };
        // So the floor is what the system actually emits at its worst.
        // (`Ord::max` is not const-callable yet, hence the runtime
        // assertion for the clamped result.)
        let worst_rung = DEFAULT_BITRATE_BPS / 2;
        assert_eq!((worst_rung * 2 / 3).max(MIN_BITRATE_BPS), MIN_BITRATE_BPS);
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let frame_size = 960; // 20ms at 48kHz
        let mut codec = OpusCodec::new(48000, 1, frame_size).unwrap();

        // Generate a simple sine wave
        let pcm: Vec<f32> = (0i16..960)
            .map(|i| {
                let t = f32::from(i);
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
