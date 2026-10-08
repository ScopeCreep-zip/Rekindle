//! macOS idle-time detection via CoreGraphics.

/// Use `CGEventSourceSecondsSinceLastEventType` from CoreGraphics.
///
/// This is the same approach used by Chromium (and thus Discord, Slack, Signal
/// Desktop, and every Electron app) for idle detection since 2012. It returns
/// seconds since the last user input event (mouse, keyboard, trackpad).
///
/// The previous `ioreg -c IOHIDSystem` approach fails to parse `HIDIdleTime`
/// on Darwin 25.x (macOS Tahoe). `CGEventSource` is a single FFI call with
/// no subprocess spawning or stdout parsing.
///
/// Reference: Chromium `ui/base/idle/idle_mac.mm`
pub(crate) fn macos_idle_seconds() -> Option<u64> {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(source_state_id: i32, event_type: u32) -> f64;
    }
    // kCGEventSourceStateCombinedSessionState = 0
    // kCGAnyInputEventType = 0xFFFFFFFF (u32::MAX)
    // SAFETY: CGEventSourceSecondsSinceLastEventType is a pure query with no side effects;
    // arguments are valid constants (source state 0, event type u32::MAX).
    let secs = unsafe { CGEventSourceSecondsSinceLastEventType(0, u32::MAX) };
    // Negative means error; NaN/Inf are also invalid
    if secs.is_finite() && secs >= 0.0 {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "f64→u64: no From impl in std; value is validated finite and non-negative"
        )]
        Some(secs as u64)
    } else {
        None
    }
}
