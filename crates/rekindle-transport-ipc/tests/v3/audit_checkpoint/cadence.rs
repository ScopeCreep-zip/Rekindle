use rekindle_transport_ipc::v3::audit::checkpoint::{CheckpointTracker, CheckpointConfig};

fn default_config() -> CheckpointConfig {
    CheckpointConfig {
        max_frames: 1024,
        max_interval_ms: 50,
    }
}

#[test]
fn checkpoint_due_after_max_frames() {
    let mut tracker = CheckpointTracker::new(default_config());
    for _ in 0..1023 {
        assert!(!tracker.is_due(), "not due before 1024 frames");
        tracker.frame_processed();
    }
    tracker.frame_processed(); // 1024th
    assert!(tracker.is_due(), "must be due at 1024 frames");
}

#[test]
fn checkpoint_due_resets_after_emit() {
    let mut tracker = CheckpointTracker::new(default_config());
    for _ in 0..1024 {
        tracker.frame_processed();
    }
    assert!(tracker.is_due());
    tracker.checkpoint_emitted();
    assert!(!tracker.is_due(), "must reset after emit");
}

#[test]
fn not_due_before_threshold() {
    let mut tracker = CheckpointTracker::new(default_config());
    for _ in 0..500 {
        tracker.frame_processed();
    }
    assert!(!tracker.is_due());
}

#[test]
fn checkpoint_seq_increments() {
    let mut tracker = CheckpointTracker::new(default_config());
    assert_eq!(tracker.next_checkpoint_seq(), 0);
    tracker.checkpoint_emitted();
    assert_eq!(tracker.next_checkpoint_seq(), 1);
    tracker.checkpoint_emitted();
    assert_eq!(tracker.next_checkpoint_seq(), 2);
}

#[test]
fn frames_since_last_checkpoint() {
    let mut tracker = CheckpointTracker::new(default_config());
    assert_eq!(tracker.frames_since_last(), 0);
    tracker.frame_processed();
    tracker.frame_processed();
    tracker.frame_processed();
    assert_eq!(tracker.frames_since_last(), 3);
    tracker.checkpoint_emitted();
    assert_eq!(tracker.frames_since_last(), 0);
}

#[test]
fn custom_config_threshold() {
    let config = CheckpointConfig {
        max_frames: 10,
        max_interval_ms: 1000,
    };
    let mut tracker = CheckpointTracker::new(config);
    for _ in 0..9 {
        tracker.frame_processed();
    }
    assert!(!tracker.is_due());
    tracker.frame_processed(); // 10th
    assert!(tracker.is_due());
}

#[test]
fn zero_frame_threshold_is_always_due() {
    let config = CheckpointConfig {
        max_frames: 0,
        max_interval_ms: 50,
    };
    let mut tracker = CheckpointTracker::new(config);
    tracker.frame_processed();
    assert!(tracker.is_due(), "threshold 0 means every frame triggers");
}
