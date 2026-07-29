//! Per-stream and per-lane credit tracking + backpressure state.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Advisory,
    Urgent,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreditScope {
    Lane,
    Stream,
}

#[derive(Debug)]
pub enum CreditError {
    Exhausted,
}

/// Tracks send credit for a single stream, optionally bounded by a lane limit.
pub struct CreditTracker {
    remaining_chunks: u32,
    generation: u64,
    lane_remaining: Option<u32>,
}

impl CreditTracker {
    /// Create a tracker with initial stream credit.
    pub fn new(initial_chunks: u32, _initial_bytes: u32) -> Self {
        Self {
            remaining_chunks: initial_chunks,
            generation: 0,
            lane_remaining: None,
        }
    }

    /// Create a tracker with both stream and lane credit limits.
    pub fn with_lane_limit(stream_chunks: u32, _stream_bytes: u32, lane_chunks: u32) -> Self {
        Self {
            remaining_chunks: stream_chunks,
            generation: 0,
            lane_remaining: Some(lane_chunks),
        }
    }

    /// Try to consume one chunk of credit.
    pub fn try_consume_chunk(&mut self) -> Result<(), CreditError> {
        if self.remaining_chunks == 0 {
            return Err(CreditError::Exhausted);
        }
        if let Some(ref mut lane) = self.lane_remaining {
            if *lane == 0 {
                return Err(CreditError::Exhausted);
            }
            *lane -= 1;
        }
        self.remaining_chunks -= 1;
        Ok(())
    }

    /// Replenish credit. Only accepted if generation is newer than current.
    pub fn replenish(&mut self, additional_chunks: u32, generation: u64) {
        if generation <= self.generation {
            return; // stale credit
        }
        self.generation = generation;
        self.remaining_chunks = additional_chunks;
    }

    pub fn remaining_chunks(&self) -> u32 {
        match self.lane_remaining {
            Some(lane) => std::cmp::min(self.remaining_chunks, lane),
            None => self.remaining_chunks,
        }
    }

    pub fn current_generation(&self) -> u64 {
        self.generation
    }
}

/// Backpressure state for a connection or stream.
#[derive(Default)]
pub struct BackpressureState {
    severity: Option<Severity>,
}

impl BackpressureState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn assert_backpressure(&mut self, severity: Severity) {
        self.severity = Some(severity);
    }

    pub fn clear(&mut self) {
        self.severity = None;
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self.severity, Some(Severity::Urgent | Severity::Critical))
    }

    pub fn may_send(&self) -> bool {
        !self.is_blocked()
    }

    pub fn current_severity(&self) -> Option<Severity> {
        self.severity
    }
}
