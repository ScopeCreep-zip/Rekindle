use super::*;
use rekindle_node::host::panic_shortcut::{PanicRoute, UNAVAILABLE};
use std::sync::atomic::{AtomicBool, Ordering};

async fn arm_with(
    shortcut: Option<&str>,
    route: PanicRoute,
) -> (Result<Done<QuickWipeState>, CommandError>, bool) {
    let armed = AtomicBool::new(false);
    let answer = arm_checked(shortcut.map(str::to_owned), async { route }, |shortcut| {
        armed.store(true, Ordering::SeqCst);
        async move {
            Ok(Done {
                value: QuickWipeState {
                    armed: true,
                    shortcut,
                },
                undo: None,
            })
        }
    })
    .await;
    (answer, armed.load(Ordering::SeqCst))
}

/// A chord this desktop cannot deliver (COSMIC without the portal) is
/// refused with the Host's reason verbatim and says nothing was armed;
/// the arming never reaches the Host, so the duress passphrase is not
/// half-armed.
#[tokio::test]
async fn an_undeliverable_chord_arms_nothing_and_says_why() {
    let (answer, armed) = arm_with(
        Some("CommandOrControl+Shift+F12"),
        PanicRoute::Unavailable(UNAVAILABLE),
    )
    .await;
    assert!(!armed, "the Host was asked to arm");
    assert_eq!(
        answer.expect_err("refused"),
        CommandError::Failed {
            code: 400,
            message: UNAVAILABLE.to_owned(),
            remediation: Some(NOTHING_ARMED.to_owned()),
        }
    );
}

/// A chord that is not one is refused the same way, and arms nothing.
#[tokio::test]
async fn an_unparsable_chord_arms_nothing() {
    let (answer, armed) = arm_with(Some("not a chord"), PanicRoute::OsGrab).await;
    assert!(!armed);
    assert!(matches!(
        answer,
        Err(CommandError::Failed { remediation: Some(r), .. }) if r == NOTHING_ARMED
    ));
}

/// The duress passphrase alone is armed whatever the desktop offers
/// (the explicit "without a shortcut" action), and a deliverable chord
/// goes to the Host with it.
#[tokio::test]
async fn without_a_chord_or_with_a_deliverable_one_the_host_arms() {
    let (answer, armed) = arm_with(None, PanicRoute::Unavailable(UNAVAILABLE)).await;
    assert!(armed && answer.is_ok());
    let (answer, armed) = arm_with(Some("CommandOrControl+Shift+F12"), PanicRoute::Portal).await;
    assert!(armed);
    assert_eq!(
        answer.expect("armed").value.shortcut.as_deref(),
        Some("CommandOrControl+Shift+F12")
    );
}
