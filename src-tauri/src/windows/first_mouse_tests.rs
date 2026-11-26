use super::accepts_first_mouse;
use crate::window_labels::WindowKind;

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
