//! Raid detection and protection for the coordinator.
//!
//! Monitors join rates and triggers defensive actions when a raid is detected.

use std::collections::VecDeque;

use rekindle_protocol::dht::community::automod::{RaidAction, RaidProtection};

/// Raid detection window in seconds.
const RAID_WINDOW_SECS: u64 = 60;

/// Current raid status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaidStatus {
    /// No raid detected.
    Normal,
    /// Raid detected, defensive actions active.
    Active,
}

/// Raid detector that monitors join rates.
pub struct RaidDetector {
    config: RaidProtection,
    /// Recent join timestamps (sliding window).
    join_window: VecDeque<u64>,
    /// Current raid status.
    status: RaidStatus,
    /// When the raid was first detected (for auto-resolve).
    raid_detected_at: Option<u64>,
    /// Whether invites are currently paused.
    pub invites_paused: bool,
    /// Whether new members are restricted to read-only.
    pub new_members_restricted: bool,
}

impl RaidDetector {
    /// Create a new detector with the given config.
    pub fn new(config: RaidProtection) -> Self {
        Self {
            config,
            join_window: VecDeque::new(),
            status: RaidStatus::Normal,
            raid_detected_at: None,
            invites_paused: false,
            new_members_restricted: false,
        }
    }

    /// Hot-reload the config.
    pub fn reload_config(&mut self, config: RaidProtection) {
        self.config = config;
    }

    /// Record a new join and check if raid threshold is exceeded.
    ///
    /// Returns `Some(actions)` if a raid was just detected (transitions from
    /// Normal to Active), or `None` if no state change occurred.
    pub fn record_join(&mut self, now_secs: u64) -> Option<Vec<RaidAction>> {
        if !self.config.enabled {
            return None;
        }

        // Prune entries outside the window
        let window_start = now_secs.saturating_sub(RAID_WINDOW_SECS);
        while self.join_window.front().is_some_and(|&t| t < window_start) {
            self.join_window.pop_front();
        }

        self.join_window.push_back(now_secs);

        // Check threshold
        if self.join_window.len() >= usize::from(self.config.join_rate_threshold)
            && self.status == RaidStatus::Normal
        {
            self.status = RaidStatus::Active;
            self.raid_detected_at = Some(now_secs);

            // Apply actions
            for action in &self.config.actions {
                match action {
                    RaidAction::PauseInvites => self.invites_paused = true,
                    RaidAction::RestrictNewMembers => self.new_members_restricted = true,
                    RaidAction::AlertOwners | RaidAction::LockdownChannels => {
                        // These are handled by the caller
                    }
                }
            }

            return Some(self.config.actions.clone());
        }

        None
    }

    /// Check if the raid should auto-resolve based on the configured timeout.
    ///
    /// Returns `true` if the raid was resolved (transitions Active -> Normal).
    pub fn check_auto_resolve(&mut self, now_secs: u64) -> bool {
        if self.status != RaidStatus::Active || self.config.auto_resolve_secs == 0 {
            return false;
        }

        if let Some(detected_at) = self.raid_detected_at {
            if now_secs.saturating_sub(detected_at) >= u64::from(self.config.auto_resolve_secs) {
                self.resolve();
                return true;
            }
        }

        false
    }

    /// Manually resolve the raid, lifting all restrictions.
    pub fn resolve(&mut self) {
        self.status = RaidStatus::Normal;
        self.raid_detected_at = None;
        self.invites_paused = false;
        self.new_members_restricted = false;
        self.join_window.clear();
    }

    /// Check if a raid is currently active.
    pub fn is_active(&self) -> bool {
        self.status == RaidStatus::Active
    }

    /// Check if join requests should be rejected (invites paused or raid active).
    pub fn should_reject_join(&self) -> bool {
        self.invites_paused || self.is_active()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config(threshold: u8, auto_resolve_secs: u32) -> RaidProtection {
        RaidProtection {
            enabled: true,
            join_rate_threshold: threshold,
            actions: vec![RaidAction::PauseInvites, RaidAction::AlertOwners],
            auto_resolve_secs,
        }
    }

    #[test]
    fn no_raid_below_threshold() {
        let mut detector = RaidDetector::new(default_config(5, 300));

        for i in 0..4 {
            let result = detector.record_join(1000 + i);
            assert!(result.is_none());
        }
        assert!(!detector.is_active());
        assert!(!detector.should_reject_join());
    }

    #[test]
    fn raid_triggers_at_threshold() {
        let mut detector = RaidDetector::new(default_config(5, 300));

        // 4 joins below threshold
        for i in 0..4 {
            detector.record_join(1000 + i);
        }
        assert!(!detector.is_active());

        // 5th join triggers raid
        let actions = detector.record_join(1004);
        assert!(actions.is_some());
        assert!(detector.is_active());
        assert!(detector.should_reject_join());
    }

    #[test]
    fn raid_does_not_retrigger() {
        let mut detector = RaidDetector::new(default_config(3, 300));

        // Trigger raid
        for i in 0..3 {
            detector.record_join(1000 + i);
        }
        assert!(detector.is_active());

        // Additional joins don't return actions again
        let result = detector.record_join(1003);
        assert!(result.is_none());
        assert!(detector.is_active());
    }

    #[test]
    fn auto_resolve() {
        let mut detector = RaidDetector::new(default_config(3, 60));

        // Trigger raid — 3rd join at t=1002 is the threshold
        for i in 0..3 {
            detector.record_join(1000 + i);
        }
        assert!(detector.is_active());

        // Not resolved at t=1050 (< 60s since detection at t=1002)
        assert!(!detector.check_auto_resolve(1050));
        assert!(detector.is_active());

        // Resolved at t=1063 (>= 60s since detection at t=1002)
        assert!(detector.check_auto_resolve(1063));
        assert!(!detector.is_active());
        assert!(!detector.should_reject_join());
    }

    #[test]
    fn manual_resolve() {
        let mut detector = RaidDetector::new(default_config(3, 0));

        // Trigger raid
        for i in 0..3 {
            detector.record_join(1000 + i);
        }
        assert!(detector.is_active());

        // Auto-resolve disabled (0 seconds)
        assert!(!detector.check_auto_resolve(9999));
        assert!(detector.is_active());

        // Manual resolve
        detector.resolve();
        assert!(!detector.is_active());
        assert!(!detector.should_reject_join());
    }

    #[test]
    fn disabled_detection() {
        let config = RaidProtection {
            enabled: false,
            join_rate_threshold: 1,
            actions: vec![RaidAction::PauseInvites],
            auto_resolve_secs: 300,
        };
        let mut detector = RaidDetector::new(config);

        // Even exceeding threshold, nothing happens when disabled
        for i in 0..10 {
            let result = detector.record_join(1000 + i);
            assert!(result.is_none());
        }
        assert!(!detector.is_active());
    }

    #[test]
    fn window_sliding() {
        let mut detector = RaidDetector::new(default_config(5, 300));

        // 4 joins at t=1000..1003 (below threshold)
        for i in 0..4 {
            detector.record_join(1000 + i);
        }
        assert!(!detector.is_active());

        // Wait for window to pass (t=1061, window is 60s)
        // 5th join at t=1061 — only 1 join in window now
        let result = detector.record_join(1061);
        assert!(result.is_none());
        assert!(!detector.is_active());
    }

    #[test]
    fn restrict_new_members_action() {
        let config = RaidProtection {
            enabled: true,
            join_rate_threshold: 3,
            actions: vec![RaidAction::RestrictNewMembers, RaidAction::PauseInvites],
            auto_resolve_secs: 300,
        };
        let mut detector = RaidDetector::new(config);

        for i in 0..3 {
            detector.record_join(1000 + i);
        }

        assert!(detector.is_active());
        assert!(detector.invites_paused);
        assert!(detector.new_members_restricted);

        detector.resolve();
        assert!(!detector.invites_paused);
        assert!(!detector.new_members_restricted);
    }
}
