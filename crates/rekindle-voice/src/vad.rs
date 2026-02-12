/// Energy-based Voice Activity Detection (VAD).
///
/// Detects whether an audio frame contains speech based on
/// the RMS energy level compared to a configurable threshold.
pub struct VoiceActivityDetector {
    /// Energy threshold (0.0–1.0). Frames above this are "speech".
    threshold: f32,
    /// Hold time in frames — keep reporting "speaking" for this many
    /// frames after the last detected speech frame.
    hold_frames: u32,
    /// Counter for hold time.
    hold_counter: u32,
    /// Whether the user is currently speaking.
    is_speaking: bool,
}

impl VoiceActivityDetector {
    /// Create a new VAD with the given threshold and hold time.
    ///
    /// - `threshold`: Energy level (0.0–1.0) above which speech is detected.
    ///   Typical values: 0.01–0.05 for sensitive, 0.1–0.2 for noisy environments.
    /// - `hold_time_ms`: How long to keep "speaking" state after last detection.
    /// - `frame_duration_ms`: Duration of each audio frame (typically 20ms for Opus).
    pub fn new(threshold: f32, hold_time_ms: u32, frame_duration_ms: u32) -> Self {
        let hold_frames = if frame_duration_ms > 0 {
            hold_time_ms / frame_duration_ms
        } else {
            0
        };
        Self {
            threshold,
            hold_frames,
            hold_counter: 0,
            is_speaking: false,
        }
    }

    /// Process a PCM audio frame and return whether the user is speaking.
    pub fn process(&mut self, samples: &[f32]) -> bool {
        let energy = rms_energy(samples);

        if energy >= self.threshold {
            self.is_speaking = true;
            self.hold_counter = self.hold_frames;
        } else if self.hold_counter > 0 {
            self.hold_counter -= 1;
        } else {
            self.is_speaking = false;
        }

        self.is_speaking
    }

    /// Check if the user is currently speaking (without processing a new frame).
    pub fn is_speaking(&self) -> bool {
        self.is_speaking
    }

    /// Set the energy threshold.
    pub fn set_threshold(&mut self, threshold: f32) {
        self.threshold = threshold.clamp(0.0, 1.0);
    }

    /// Get the current threshold.
    pub fn threshold(&self) -> f32 {
        self.threshold
    }
}

/// Compute the Root Mean Square energy of a PCM buffer.
fn rms_energy(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    #[allow(clippy::cast_precision_loss)] // sample counts will never exceed f32 mantissa range in practice
    let len = samples.len() as f32;
    (sum_sq / len).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_silence_not_speaking() {
        let mut vad = VoiceActivityDetector::new(0.01, 100, 20);
        let silence = vec![0.0f32; 960];
        assert!(!vad.process(&silence));
    }

    #[test]
    fn test_loud_signal_speaking() {
        let mut vad = VoiceActivityDetector::new(0.01, 100, 20);
        #[allow(clippy::cast_precision_loss)]
        let loud: Vec<f32> = (0..960).map(|i: i32| (i as f32 * 0.01).sin() * 0.5).collect();
        assert!(vad.process(&loud));
    }

    #[test]
    fn test_hold_time() {
        let mut vad = VoiceActivityDetector::new(0.01, 60, 20); // 3 frames hold
        let loud: Vec<f32> = vec![0.5; 960];
        let silence = vec![0.0f32; 960];

        vad.process(&loud); // speaking
        assert!(vad.is_speaking());

        // Should hold for 3 silent frames
        assert!(vad.process(&silence)); // hold_counter = 2
        assert!(vad.process(&silence)); // hold_counter = 1
        assert!(vad.process(&silence)); // hold_counter = 0
        assert!(!vad.process(&silence)); // now not speaking
    }

    #[test]
    fn test_rms_energy() {
        assert_eq!(rms_energy(&[]), 0.0);
        let signal = vec![0.5f32; 100];
        let energy = rms_energy(&signal);
        assert!((energy - 0.5).abs() < 0.001);
    }
}
