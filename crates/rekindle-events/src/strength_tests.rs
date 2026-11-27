use super::*;

/// L8: a 2 s dip does not move the bars; a rise shows at once; a fall
/// that lasts the hold shows when it ends.
#[test]
fn a_short_dip_holds_and_a_lasting_fall_shows_after_the_hold() {
    let mut w = StrengthWindow::default();
    assert_eq!(w.sample(0, 4), Some(4), "a rise shows at once");
    assert_eq!(w.sample(1_000, 1), None, "the dip is held");
    assert!(w.holding());
    assert_eq!(w.sample(3_000, 4), None, "back within the hold");
    assert!(!w.holding());
    assert_eq!(w.settle(3_000 + HOLD_MS), None, "the dip never shows");

    assert_eq!(w.sample(10_000, 2), None);
    assert_eq!(w.settle(10_000 + HOLD_MS - 1), None, "still held");
    assert_eq!(w.settle(10_000 + HOLD_MS), Some(2), "the fall shows");
    assert_eq!(w.sample(20_000, 5), Some(5));
    assert_eq!(w.shown(), 5);
}

/// The host's clock on tokio's paused timeline, so a hold elapses
/// without waiting; it starts at a wall-clock-like instant, as a real
/// clock never reads within the first hold of the epoch.
struct PausedClock(tokio::time::Instant);

const EPOCH_OFFSET_MS: u64 = 1_800_000_000_000;

impl rekindle_utils::Clock for PausedClock {
    fn now_ms(&self) -> u64 {
        EPOCH_OFFSET_MS + u64::try_from(self.0.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// The driver both hosts run (plan E7.2: one rule, two hosts until
/// F1): a dip shorter than the hold never publishes, a lasting fall
/// publishes when its hold ends.
#[tokio::test(start_paused = true)]
async fn a_dip_shorter_than_the_hold_never_publishes() {
    use rekindle_types::notification::AttachmentState;
    let strength = Strength::new(Arc::new(PausedClock(tokio::time::Instant::now())));
    let full = AttachmentState::FullyAttached.bars();
    let weak = AttachmentState::AttachedWeak.bars();

    assert_eq!(strength.report("fully_attached").publish, Some(full));
    let dip = strength.report("attached_weak");
    assert_eq!(
        dip,
        Report {
            publish: None,
            hold: true
        }
    );
    tokio::time::sleep(std::time::Duration::from_millis(2_000)).await;
    assert_eq!(
        strength.report("fully_attached"),
        Report {
            publish: None,
            hold: false
        }
    );
    assert_eq!(
        strength.settle_held().await,
        None,
        "the 2 s dip never shows"
    );
    assert_eq!(strength.shown(), full);

    assert!(strength.report("attached_weak").hold);
    assert_eq!(
        strength.settle_held().await,
        Some(weak),
        "a lasting fall shows"
    );
}
