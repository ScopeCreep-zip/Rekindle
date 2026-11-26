use super::{accepts_first_mouse, background_policy};
use crate::window_labels::WindowKind;
use tauri::utils::config::BackgroundThrottlingPolicy;

/// Every window drawn from a skin tree acts on the click that activates it;
/// the app's own windows keep the platform's behaviour.
#[test]
fn skin_windows_act_on_the_first_click() {
    for kind in WindowKind::ALL {
        let skin = matches!(
            kind,
            WindowKind::BuddyList
                | WindowKind::Chat
                | WindowKind::Community
                | WindowKind::SkinPopup
        );
        assert_eq!(accepts_first_mouse(kind), skin, "{kind:?}");
    }
}

/// A window that can carry live media is never throttled in the background
/// (its page decodes video and answers keyframe and consent requests); the
/// others are throttled, not suspended.
#[test]
fn media_windows_are_never_throttled() {
    for kind in WindowKind::ALL {
        let media = matches!(
            kind,
            WindowKind::Call | WindowKind::Chat | WindowKind::Community
        );
        let want = if media {
            BackgroundThrottlingPolicy::Disabled
        } else {
            BackgroundThrottlingPolicy::Throttle
        };
        assert_eq!(background_policy(kind), want, "{kind:?}");
    }
}
