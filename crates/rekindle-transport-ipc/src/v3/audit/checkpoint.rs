//! Checkpoint cadence tracking and data structures.

/// Checkpoint emission configuration.
#[derive(Debug, Clone, Copy)]
pub struct CheckpointConfig {
    pub max_frames: u64,
    pub max_interval_ms: u64,
}

impl Default for CheckpointConfig {
    fn default() -> Self {
        Self { max_frames: 1024, max_interval_ms: 50 }
    }
}

/// Data carried in an AUDIT_CHECKPOINT frame.
#[derive(Debug, Clone)]
pub struct CheckpointData {
    pub chain_index: u64,
    pub chain_length: u64,
    pub chain_link: [u8; 32],
    pub anchor_link: [u8; 32],
    pub checkpoint_seq: u64,
}

/// Tracks whether a checkpoint is due based on frame count and time.
pub struct CheckpointTracker {
    config: CheckpointConfig,
    frames_since_last: u64,
    next_seq: u64,
}

impl CheckpointTracker {
    pub fn new(config: CheckpointConfig) -> Self {
        Self {
            config,
            frames_since_last: 0,
            next_seq: 0,
        }
    }

    /// Record that a frame was processed.
    pub fn frame_processed(&mut self) {
        self.frames_since_last += 1;
    }

    /// Check if a checkpoint should be emitted.
    /// Frame-count threshold only; time-based checking is the caller's
    /// responsibility via wall-clock comparison.
    pub fn is_due(&self) -> bool {
        if self.config.max_frames == 0 {
            return self.frames_since_last > 0;
        }
        self.frames_since_last >= self.config.max_frames
    }

    /// Record that a checkpoint was emitted.
    pub fn checkpoint_emitted(&mut self) {
        self.frames_since_last = 0;
        self.next_seq += 1;
    }

    pub fn next_checkpoint_seq(&self) -> u64 {
        self.next_seq
    }

    pub fn frames_since_last(&self) -> u64 {
        self.frames_since_last
    }
}
