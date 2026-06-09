//! Sender-side frame retention buffer for AUDIT_REPLAY.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use crate::v3::bulk::counters;

#[derive(Debug, Clone)]
pub struct RetentionConfig {
    pub max_frames: usize,
    pub max_bytes: usize,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self { max_frames: 65536, max_bytes: 256 * 1024 * 1024 }
    }
}

/// Bounded retention buffer keyed by session_seq.
/// Evicts oldest entries when count or byte limits are exceeded.
pub struct RetentionBuffer {
    config: RetentionConfig,
    frames: BTreeMap<u64, Vec<u8>>,
    total_bytes: usize,
}

impl RetentionBuffer {
    pub fn new(config: RetentionConfig) -> Self {
        Self {
            config,
            frames: BTreeMap::new(),
            total_bytes: 0,
        }
    }

    /// Store a frame by session_seq. Evicts oldest if limits exceeded.
    pub fn store(&mut self, session_seq: u64, frame: Vec<u8>) {
        let frame_size = frame.len();
        self.frames.insert(session_seq, frame);
        self.total_bytes += frame_size;

        counters::DIAG_RETENTION_STORED_BYTES.fetch_add(frame_size as u64, Ordering::Relaxed);
        counters::DIAG_RETENTION_STORED_FRAMES.fetch_add(1, Ordering::Relaxed);

        self.evict();
    }

    /// Retrieve a frame by session_seq.
    pub fn get(&self, session_seq: u64) -> Option<&[u8]> {
        self.frames.get(&session_seq).map(Vec::as_slice)
    }

    /// Retrieve frames for a range, returning None for missing entries.
    pub fn get_range(&self, start: u64, end: u64) -> Vec<Option<&[u8]>> {
        (start..=end)
            .map(|seq| self.frames.get(&seq).map(Vec::as_slice))
            .collect()
    }

    pub fn remove(&mut self, session_seq: u64) -> bool {
        if let Some(removed) = self.frames.remove(&session_seq) {
            self.total_bytes = self.total_bytes.saturating_sub(removed.len());
            true
        } else {
            false
        }
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    fn evict(&mut self) {
        // Evict by count
        while self.frames.len() > self.config.max_frames {
            if let Some((&oldest_seq, _)) = self.frames.iter().next() {
                if let Some(removed) = self.frames.remove(&oldest_seq) {
                    self.total_bytes = self.total_bytes.saturating_sub(removed.len());
                }
            } else {
                break;
            }
        }

        // Evict by bytes
        while self.total_bytes > self.config.max_bytes {
            if let Some((&oldest_seq, _)) = self.frames.iter().next() {
                if let Some(removed) = self.frames.remove(&oldest_seq) {
                    self.total_bytes = self.total_bytes.saturating_sub(removed.len());
                }
            } else {
                break;
            }
        }
    }
}
