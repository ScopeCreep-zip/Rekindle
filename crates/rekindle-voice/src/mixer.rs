/// Audio mixer for combining multiple voice streams in group calls.
pub struct AudioMixer {
    /// Number of output channels (typically 1 for mono, 2 for stereo).
    output_channels: u16,
    /// Volume scaling per participant (keyed by public key hex).
    volumes: std::collections::HashMap<String, f32>,
}

impl AudioMixer {
    /// Create a new mixer with the given output channel count.
    pub fn new(output_channels: u16) -> Self {
        Self {
            output_channels,
            volumes: std::collections::HashMap::new(),
        }
    }

    /// Mix multiple decoded PCM streams into a single output buffer.
    ///
    /// Each entry in `streams` is (`participant_id`, `pcm_samples`).
    /// Returns the mixed output at the same sample count as the longest input.
    pub fn mix(&self, streams: &[(&str, &[f32])]) -> Vec<f32> {
        if streams.is_empty() {
            return Vec::new();
        }

        let max_len = streams.iter().map(|(_, s)| s.len()).max().unwrap_or(0);
        let mut output = vec![0.0f32; max_len];

        for (participant_id, samples) in streams {
            let volume = self.volumes.get(*participant_id).copied().unwrap_or(1.0);
            for (i, &sample) in samples.iter().enumerate() {
                output[i] += sample * volume;
            }
        }

        // Soft clamp to prevent clipping
        for sample in &mut output {
            *sample = sample.clamp(-1.0, 1.0);
        }

        output
    }

    /// Set the volume for a participant (0.0 = mute, 1.0 = full).
    pub fn set_volume(&mut self, participant_id: &str, volume: f32) {
        self.volumes
            .insert(participant_id.to_string(), volume.clamp(0.0, 2.0));
    }

    /// Remove a participant from the mixer.
    pub fn remove_participant(&mut self, participant_id: &str) {
        self.volumes.remove(participant_id);
    }

    pub fn output_channels(&self) -> u16 {
        self.output_channels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mix_two_streams() {
        let mixer = AudioMixer::new(1);
        let a = [0.5f32, 0.3, -0.2];
        let b = [0.2f32, -0.1, 0.4];
        let result = mixer.mix(&[("a", &a), ("b", &b)]);
        assert_eq!(result.len(), 3);
        assert!((result[0] - 0.7).abs() < 0.001);
        assert!((result[1] - 0.2).abs() < 0.001);
        assert!((result[2] - 0.2).abs() < 0.001);
    }

    #[test]
    fn test_mix_clamps() {
        let mixer = AudioMixer::new(1);
        let a = [0.9f32];
        let b = [0.9f32];
        let result = mixer.mix(&[("a", &a), ("b", &b)]);
        assert_eq!(result[0], 1.0); // clamped
    }
}
