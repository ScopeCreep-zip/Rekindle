#[cfg(test)]
mod wait_until_applied_tests {
    use crate::platform::executor::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Notify;

    const SHORT_TIMEOUT: Duration = Duration::from_millis(50);

    #[tokio::test]
    async fn returns_at_once_when_already_applied() {
        let notify = Notify::new();
        assert!(wait_until_applied(&notify, SHORT_TIMEOUT, || true).await);
    }

    #[tokio::test]
    async fn a_stale_event_does_not_end_the_wait() {
        let notify = Notify::new();
        notify.notify_one();
        assert!(!wait_until_applied(&notify, SHORT_TIMEOUT, || false).await);
    }

    #[tokio::test]
    async fn ends_when_a_later_event_finds_the_change_applied() {
        let notify = Arc::new(Notify::new());
        let applied = Arc::new(AtomicBool::new(false));
        notify.notify_one();
        let landing = tokio::spawn({
            let notify = Arc::clone(&notify);
            let applied = Arc::clone(&applied);
            async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                applied.store(true, Ordering::SeqCst);
                notify.notify_one();
            }
        });

        let is_applied = || applied.load(Ordering::SeqCst);
        assert!(wait_until_applied(&notify, Duration::from_secs(5), is_applied).await);
        assert!(is_applied(), "returned before the change landed");
        landing.await.unwrap();
    }
}

#[cfg(test)]
mod window_rect_tests {
    use crate::platform::executor::*;

    #[test]
    fn converts_physical_window_rect_at_common_scale_factors() {
        let cases = [
            (
                PhysicalPosition::new(-200, 150),
                PhysicalSize::new(720, 540),
                1.0,
                WindowRect {
                    x: -200,
                    y: 150,
                    width: 720,
                    height: 540,
                },
            ),
            (
                PhysicalPosition::new(-151, 251),
                PhysicalSize::new(1000, 750),
                1.25,
                WindowRect {
                    x: -121,
                    y: 201,
                    width: 800,
                    height: 600,
                },
            ),
            (
                PhysicalPosition::new(-400, 300),
                PhysicalSize::new(1440, 1080),
                2.0,
                WindowRect {
                    x: -200,
                    y: 150,
                    width: 720,
                    height: 540,
                },
            ),
        ];

        for (position, size, scale_factor, expected) in cases {
            let actual = physical_window_rect_to_logical(position, size, scale_factor);

            assert_eq!(actual.x, expected.x);
            assert_eq!(actual.y, expected.y);
            assert_eq!(actual.width, expected.width);
            assert_eq!(actual.height, expected.height);
        }
    }

    #[test]
    fn converts_physical_window_chrome_before_subtracting_it() {
        let retina_chrome = physical_window_chrome_to_logical(
            PhysicalSize::new(1440, 1080),
            PhysicalSize::new(1440, 1032),
            2.0,
        );
        let fractional_chrome = physical_window_chrome_to_logical(
            PhysicalSize::new(1251, 1001),
            PhysicalSize::new(1000, 750),
            1.25,
        );

        assert_eq!(retina_chrome, LogicalSize::new(0, 24));
        assert_eq!(fractional_chrome, LogicalSize::new(201, 201));
    }
}
