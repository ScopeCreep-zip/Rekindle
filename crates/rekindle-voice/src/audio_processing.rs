use nnnoiseless::DenoiseState;

/// Result of processing a capture frame through the audio pipeline.
pub struct ProcessedFrame {
    /// Cleaned PCM samples (normalized [-1.0, 1.0]).
    pub samples: Vec<f32>,
    /// Whether the frame contains speech (VAD result).
    pub is_speech: bool,
}

/// Newtype wrapper to allow `VoipAec3` to be used in async tasks.
///
/// `VoipAec3` is `!Send + !Sync` due to internal raw pointers, but it is only
/// ever accessed from a single task at a time (the voice send loop). The tokio
/// runtime may migrate the task between OS threads, which requires `Send`.
/// This is safe because no concurrent access occurs.
struct SendableAec3(aec3::voip::VoipAec3);

// SAFETY: VoipAec3 is !Send due to internal raw pointers, but we only ever
// access it from a single tokio task (voice send loop). The runtime may migrate
// the task between threads but never accesses it concurrently.
unsafe impl Send for SendableAec3 {}

/// Audio processor that chains echo cancellation, noise suppression, and VAD.
///
/// Pipeline per 10ms sub-frame:
/// ```text
/// Raw mic (normalized) → scale to 16-bit range
///     → AEC3.process() (if enabled + speaker ref available)
///     → nnnoiseless.process_frame() → returns (denoised, vad_probability)
///     → scale back to normalized
///     → VAD gate using vad_probability vs threshold
/// ```
///
/// nnnoiseless operates on 480-sample frames (10ms at 48kHz). Our Opus frames
/// are 960 samples (20ms), so we process two sub-frames per call.
pub struct AudioProcessor {
    denoiser: Box<DenoiseState<'static>>,
    echo_canceller: Option<SendableAec3>,
    /// Whether noise suppression is enabled (can be toggled at runtime).
    noise_suppression_enabled: bool,
    /// Whether echo cancellation is enabled.
    echo_cancellation_enabled: bool,
    /// nnnoiseless VAD probability threshold (0.0–1.0).
    vad_threshold: f32,
    /// Hold time in frames — keep "speaking" state for this many frames after last detection.
    vad_hold_frames: u32,
    vad_hold_counter: u32,
    is_speaking: bool,
}

const SAMPLE_RATE: usize = 48000;
const CHANNELS: usize = 1;

impl AudioProcessor {
    /// Create a new audio processor.
    ///
    /// - `noise_suppression`: whether to enable `RNNoise` denoising
    /// - `echo_cancellation`: whether to enable AEC3 echo cancellation
    /// - `vad_threshold`: nnnoiseless VAD probability threshold (0.0–1.0)
    /// - `vad_hold_ms`: how long to sustain "speaking" state after last detection
    /// - `frame_duration_ms`: duration of each Opus frame (typically 20ms)
    pub fn new(
        noise_suppression: bool,
        echo_cancellation: bool,
        vad_threshold: f32,
        vad_hold_ms: u32,
        frame_duration_ms: u32,
    ) -> Self {
        let hold_frames = if frame_duration_ms > 0 {
            vad_hold_ms / frame_duration_ms
        } else {
            0
        };

        let echo_canceller = if echo_cancellation {
            match aec3::voip::VoipAec3::builder(SAMPLE_RATE, CHANNELS, CHANNELS).build() {
                Ok(aec) => Some(SendableAec3(aec)),
                Err(e) => {
                    tracing::warn!(error = ?e, "failed to create AEC3 — echo cancellation disabled");
                    None
                }
            }
        } else {
            None
        };

        Self {
            denoiser: DenoiseState::new(),
            echo_canceller,
            noise_suppression_enabled: noise_suppression,
            echo_cancellation_enabled: echo_cancellation,
            vad_threshold,
            vad_hold_frames: hold_frames,
            vad_hold_counter: 0,
            is_speaking: false,
        }
    }

    /// Process a capture frame (960 samples at 48kHz = 20ms).
    ///
    /// Applies echo cancellation (if enabled and speaker reference provided),
    /// noise suppression, and VAD. Returns cleaned audio and speech detection result.
    ///
    /// The 960-sample frame is split into two 480-sample sub-frames. Each sub-frame
    /// runs the full pipeline. VAD probability is averaged across both sub-frames.
    pub fn process_capture(
        &mut self,
        input: &[f32],
        speaker_ref: Option<&[f32]>,
    ) -> ProcessedFrame {
        let sub_frame_size = DenoiseState::FRAME_SIZE; // 480
        let num_sub_frames = input.len() / sub_frame_size;

        let mut output = Vec::with_capacity(input.len());
        let mut vad_prob_sum = 0.0f32;

        for i in 0..num_sub_frames {
            let start = i * sub_frame_size;
            let end = start + sub_frame_size;
            let sub_input = &input[start..end];

            // Get matching speaker reference sub-frame (if available)
            let sub_speaker_ref = speaker_ref.and_then(|sr| {
                let sr_end = end.min(sr.len());
                if start < sr_end {
                    Some(&sr[start..sr_end])
                } else {
                    None
                }
            });

            let (processed, vad_prob) = self.process_sub_frame(sub_input, sub_speaker_ref);
            output.extend_from_slice(&processed);
            vad_prob_sum += vad_prob;
        }

        // Handle any remaining samples that don't fit a sub-frame
        let processed_len = num_sub_frames * sub_frame_size;
        if processed_len < input.len() {
            output.extend_from_slice(&input[processed_len..]);
        }

        // Average VAD probability across sub-frames
        let avg_vad_prob = if num_sub_frames > 0 {
            #[allow(clippy::cast_precision_loss)]
            let avg = vad_prob_sum / num_sub_frames as f32;
            avg
        } else {
            0.0
        };

        let is_speech = self.update_vad_state(avg_vad_prob);

        ProcessedFrame {
            samples: output,
            is_speech,
        }
    }

    /// Process a single 480-sample (10ms) sub-frame through the full pipeline.
    ///
    /// Returns (`output_samples`, `vad_probability`).
    fn process_sub_frame(
        &mut self,
        input: &[f32],
        speaker_ref: Option<&[f32]>,
    ) -> (Vec<f32>, f32) {
        let sub_frame_size = DenoiseState::FRAME_SIZE;

        // Step 1: Echo cancellation (operates on normalized [-1, 1] floats)
        let echo_cancelled = if self.echo_cancellation_enabled {
            if let Some(ref mut aec) = self.echo_canceller {
                let mut aec_output = vec![0.0f32; sub_frame_size];

                // Feed render/speaker reference and process capture in one call
                match aec.0.process(
                    input,
                    speaker_ref,
                    false, // level_change
                    &mut aec_output,
                ) {
                    Ok(_metrics) => aec_output,
                    Err(e) => {
                        tracing::trace!(error = ?e, "AEC3 process failed — passing through");
                        input.to_vec()
                    }
                }
            } else {
                input.to_vec()
            }
        } else {
            input.to_vec()
        };

        // Step 2: Scale to 16-bit PCM range for nnnoiseless
        // nnnoiseless expects [-32768.0, 32767.0], not normalized [-1.0, 1.0]
        let mut scaled_input = [0.0f32; DenoiseState::FRAME_SIZE];
        for (i, &sample) in echo_cancelled.iter().take(sub_frame_size).enumerate() {
            scaled_input[i] = sample * 32767.0;
        }

        // Step 3: Noise suppression + VAD
        let mut denoised = [0.0f32; DenoiseState::FRAME_SIZE];
        let vad_prob = if self.noise_suppression_enabled {
            self.denoiser.process_frame(&mut denoised, &scaled_input)
        } else {
            // Even without noise suppression, run through denoiser for VAD probability
            let prob = self.denoiser.process_frame(&mut denoised, &scaled_input);
            // But use the original (scaled) audio, not the denoised version
            denoised.copy_from_slice(&scaled_input);
            prob
        };

        // Step 4: Scale back to normalized [-1.0, 1.0]
        let mut output = vec![0.0f32; sub_frame_size];
        for (i, &sample) in denoised.iter().take(sub_frame_size).enumerate() {
            output[i] = (sample / 32767.0).clamp(-1.0, 1.0);
        }

        (output, vad_prob)
    }

    /// Update VAD state based on nnnoiseless probability.
    fn update_vad_state(&mut self, vad_probability: f32) -> bool {
        if vad_probability >= self.vad_threshold {
            self.is_speaking = true;
            self.vad_hold_counter = self.vad_hold_frames;
        } else if self.vad_hold_counter > 0 {
            self.vad_hold_counter -= 1;
        } else {
            self.is_speaking = false;
        }

        self.is_speaking
    }

    /// Feed speaker/playback reference audio to AEC3's render side.
    ///
    /// Should be called with the mixed playback audio (before sending to speakers)
    /// so AEC3 can model the echo path. Input should be split into 10ms sub-frames.
    pub fn feed_speaker_reference(&mut self, speaker_audio: &[f32]) {
        if !self.echo_cancellation_enabled {
            return;
        }
        let Some(ref mut aec) = self.echo_canceller else {
            return;
        };

        let sub_frame_size = DenoiseState::FRAME_SIZE; // 480
        let mut pos = 0;
        while pos + sub_frame_size <= speaker_audio.len() {
            let sub_frame = &speaker_audio[pos..pos + sub_frame_size];
            if let Err(e) = aec.0.handle_render_frame(sub_frame) {
                tracing::trace!(error = ?e, "AEC3 render feed failed");
            }
            pos += sub_frame_size;
        }
    }

    /// Reset all internal state (on reconnect or device change).
    pub fn reset(&mut self) {
        self.denoiser = DenoiseState::new();
        self.vad_hold_counter = 0;
        self.is_speaking = false;

        // Re-create AEC3 if it was enabled
        if self.echo_cancellation_enabled {
            self.echo_canceller =
                aec3::voip::VoipAec3::builder(SAMPLE_RATE, CHANNELS, CHANNELS)
                    .build()
                    .ok()
                    .map(SendableAec3);
        }
    }

    /// Enable or disable noise suppression at runtime.
    pub fn set_noise_suppression(&mut self, enabled: bool) {
        self.noise_suppression_enabled = enabled;
    }

    /// Update the VAD threshold.
    pub fn set_vad_threshold(&mut self, threshold: f32) {
        self.vad_threshold = threshold.clamp(0.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_silence_not_speech() {
        let mut proc = AudioProcessor::new(false, false, 0.5, 100, 20);
        let silence = vec![0.0f32; 960];
        let result = proc.process_capture(&silence, None);
        assert!(!result.is_speech);
        assert_eq!(result.samples.len(), 960);
    }

    #[test]
    fn test_noise_suppression_scaling() {
        // Verify output is normalized [-1, 1] despite internal 16-bit scaling
        let mut proc = AudioProcessor::new(true, false, 0.01, 100, 20);
        let signal: Vec<f32> = (0..960)
            .map(|i: i32| {
                #[allow(clippy::cast_precision_loss)]
                let t = i as f32;
                (t * 440.0 * std::f32::consts::TAU / 48000.0).sin() * 0.3
            })
            .collect();
        let result = proc.process_capture(&signal, None);
        assert_eq!(result.samples.len(), 960);

        // All output samples must be in normalized range
        for &sample in &result.samples {
            assert!(
                (-1.0..=1.0).contains(&sample),
                "sample {sample} out of normalized range"
            );
        }
    }

    #[test]
    fn test_with_echo_cancellation() {
        let mut proc = AudioProcessor::new(true, true, 0.5, 100, 20);
        let silence = vec![0.0f32; 960];
        let speaker = vec![0.0f32; 960];

        // Feed speaker reference
        proc.feed_speaker_reference(&speaker);

        // Process with speaker reference — should not panic
        let result = proc.process_capture(&silence, Some(&speaker));
        assert_eq!(result.samples.len(), 960);
    }
}
