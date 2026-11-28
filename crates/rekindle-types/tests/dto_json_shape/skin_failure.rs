//! `SkinApplyFailure`'s served words (thin frontend).

use rekindle_types::dto::skin_library::SkinApplyFailure;

/// Every skin failure a window reports has the Host's words (thin
/// frontend: the window shows `notice` as served, never its own copy).
#[test]
fn every_skin_failure_cause_has_the_hosts_words() {
    let load = "Rekindle could not load the skin for this window.";
    assert_eq!(SkinApplyFailure::LOAD_FAILED_MESSAGE, load);
    assert_eq!(
        SkinApplyFailure::ImageDecode {
            path: "skin://x/a.png".into()
        }
        .notice(),
        load
    );
    assert_eq!(
        SkinApplyFailure::SheetError {
            detail: "404".into()
        }
        .notice(),
        load
    );
    assert_eq!(
        SkinApplyFailure::WindowTooSmall {
            width: 470,
            height: 380
        }
        .notice(),
        "This skin needs a window at least 470 × 380 pixels."
    );
    assert_eq!(
        SkinApplyFailure::load_timeout().notice(),
        SkinApplyFailure::LOAD_TIMEOUT_MESSAGE
    );
}
