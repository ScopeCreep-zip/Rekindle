//! Fallback tracking — disables handoff after consecutive failures.

#[derive(Debug, Clone)]
pub struct FallbackConfig {
    pub max_consecutive_failures: u32,
}

impl Default for FallbackConfig {
    fn default() -> Self {
        Self { max_consecutive_failures: 3 }
    }
}

pub struct FallbackTracker {
    config: FallbackConfig,
    consecutive: u32,
}

impl FallbackTracker {
    pub fn new(config: FallbackConfig) -> Self {
        Self { config, consecutive: 0 }
    }

    pub fn record_success(&mut self) {
        self.consecutive = 0;
    }

    pub fn record_failure(&mut self) {
        self.consecutive += 1;
    }

    pub fn is_disabled(&self) -> bool {
        self.consecutive >= self.config.max_consecutive_failures
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive
    }

    pub fn reset(&mut self) {
        self.consecutive = 0;
    }
}
