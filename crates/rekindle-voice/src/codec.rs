use crate::error::VoiceError;

/// Starting Opus bitrate, matching the per-channel default the major
/// voice services ship. The route allocator sets the rate from
/// [`MIN_BITRATE_BPS`] up to this ceiling, with audio taking priority.
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
        // voice channel. The route allocator lowers the rate when bandwidth is limited.
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
        // the allocator's floor is kept well clear of that region.
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

    /// Apply the audio target from the route allocator.
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
#[path = "codec_tests.rs"]
mod tests;
