use super::*;

#[test]
fn push_and_count() {
    let mut stack = NotificationStack::new();
    stack.push("a".into(), ToastLevel::Info);
    stack.push("b".into(), ToastLevel::Info);
    assert_eq!(stack.toasts.len(), 2);
}

#[test]
fn dedup_identical_within_window() {
    let mut stack = NotificationStack::new();
    stack.push("hello".into(), ToastLevel::Info);
    stack.push("hello".into(), ToastLevel::Info);
    assert_eq!(stack.toasts.len(), 1);
}

#[test]
fn no_dedup_different_messages() {
    let mut stack = NotificationStack::new();
    stack.push("a".into(), ToastLevel::Info);
    stack.push("b".into(), ToastLevel::Info);
    assert_eq!(stack.toasts.len(), 2);
}

#[test]
fn evicts_oldest_at_capacity() {
    let mut stack = NotificationStack::new();
    stack.push("a".into(), ToastLevel::Error);
    stack.push("b".into(), ToastLevel::Error);
    stack.push("c".into(), ToastLevel::Error);
    stack.push("d".into(), ToastLevel::Error);
    assert_eq!(stack.toasts.len(), MAX_VISIBLE);
    // Newest should be at front
    assert_eq!(stack.toasts.front().unwrap().message, "d");
}

#[test]
fn tick_removes_expired_info() {
    let mut stack = NotificationStack::new();
    stack.toasts.push_front(Toast {
        message: "old".into(),
        level: ToastLevel::Info,
        created_at: Instant::now()
            .checked_sub(Duration::from_secs(10))
            .unwrap_or_else(Instant::now),
    });
    stack.toasts.push_front(Toast {
        message: "new".into(),
        level: ToastLevel::Info,
        created_at: Instant::now(),
    });
    stack.tick();
    assert_eq!(stack.toasts.len(), 1);
    assert_eq!(stack.toasts.front().unwrap().message, "new");
}

#[test]
fn tick_keeps_sticky_errors() {
    let mut stack = NotificationStack::new();
    stack.toasts.push_front(Toast {
        message: "error".into(),
        level: ToastLevel::Error,
        created_at: Instant::now()
            .checked_sub(Duration::from_secs(60))
            .unwrap_or_else(Instant::now),
    });
    stack.tick();
    assert_eq!(stack.toasts.len(), 1);
}

#[test]
fn tick_keeps_sticky_warnings() {
    let mut stack = NotificationStack::new();
    stack.toasts.push_front(Toast {
        message: "warn".into(),
        level: ToastLevel::Warning,
        created_at: Instant::now()
            .checked_sub(Duration::from_secs(60))
            .unwrap_or_else(Instant::now),
    });
    stack.tick();
    assert_eq!(stack.toasts.len(), 1);
}

#[test]
fn dismiss_oldest_removes_bottom() {
    let mut stack = NotificationStack::new();
    stack.push("first".into(), ToastLevel::Error);
    stack.push("second".into(), ToastLevel::Error);
    stack.dismiss_oldest();
    assert_eq!(stack.toasts.len(), 1);
    assert_eq!(stack.toasts.front().unwrap().message, "second");
}

#[test]
fn is_empty_works() {
    let mut stack = NotificationStack::new();
    assert!(stack.is_empty());
    stack.push("a".into(), ToastLevel::Info);
    assert!(!stack.is_empty());
}
