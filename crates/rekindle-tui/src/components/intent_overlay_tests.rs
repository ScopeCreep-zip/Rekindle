use super::*;
use crossterm::event::KeyModifiers;
use rekindle_types::intent::{IntentClass, IntentId, IntentKind, Notifies, Reversibility, Subject};

fn review(proof: ProofRequired) -> IntentReview {
    IntentReview {
        intent_id: IntentId::v7(1_000, [7; 10]),
        kind: IntentKind::LeaveCommunity,
        class: IntentClass::Destructive,
        subject: Subject::Community {
            id: "c".into(),
            name: "Team".into(),
        },
        facts: Vec::new(),
        escalations: Vec::new(),
        reversibility: Reversibility::NeedsOthers,
        notifies: Notifies::Members(3),
        proof,
        expires_at_ms: 1,
        acknowledgement: None,
        cancel_sends: false,
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn cancel_has_the_focus_and_esc_cancels() {
    let mut overlay = IntentOverlay::default();
    overlay.show(review(ProofRequired::Commit));
    assert!(matches!(
        overlay.handle_key(key(KeyCode::Enter)),
        Some(Action::CancelIntent { .. })
    ));
    assert!(!overlay.visible());
    overlay.show(review(ProofRequired::Commit));
    assert!(matches!(
        overlay.handle_key(key(KeyCode::Esc)),
        Some(Action::CancelIntent { .. })
    ));
}

#[test]
fn esc_does_not_cancel_a_review_whose_cancel_sends() {
    let mut overlay = IntentOverlay::default();
    let mut held = review(ProofRequired::Commit);
    held.cancel_sends = true;
    overlay.show(held);
    assert!(overlay.handle_key(key(KeyCode::Esc)).is_none());
    assert!(overlay.visible());
    assert!(matches!(
        overlay.handle_key(key(KeyCode::Enter)),
        Some(Action::CancelIntent { .. })
    ));
}

#[test]
fn a_typed_subject_is_committed_as_typed() {
    let mut overlay = IntentOverlay::default();
    overlay.show(review(ProofRequired::TypeSubject));
    overlay.handle_key(key(KeyCode::Tab));
    // Nothing typed yet: Commit does nothing.
    assert!(overlay.handle_key(key(KeyCode::Enter)).is_none());
    for c in "Team".chars() {
        overlay.handle_key(key(KeyCode::Char(c)));
    }
    match overlay.handle_key(key(KeyCode::Enter)) {
        Some(Action::CommitIntent { proof, .. }) => {
            assert_eq!(proof, IntentProof::Typed("Team".into()));
        }
        other => panic!("expected a commit, got {other:?}"),
    }
}

#[test]
fn acknowledge_needs_the_checkbox() {
    let mut overlay = IntentOverlay::default();
    overlay.show(review(ProofRequired::Acknowledge));
    overlay.handle_key(key(KeyCode::Tab));
    assert!(overlay.handle_key(key(KeyCode::Enter)).is_none());
    overlay.handle_key(key(KeyCode::Char(' ')));
    assert!(matches!(
        overlay.handle_key(key(KeyCode::Enter)),
        Some(Action::CommitIntent {
            proof: IntentProof::Acknowledged,
            ..
        })
    ));
}

#[test]
fn centered_rect_centers_and_clamps() {
    let popup = centered_rect(Rect::new(0, 0, 80, 24), 40, 10);
    assert_eq!(
        (popup.x, popup.y, popup.width, popup.height),
        (20, 7, 40, 10)
    );
    let popup = centered_rect(Rect::new(0, 0, 20, 8), 50, 20);
    assert_eq!((popup.width, popup.height), (20, 8));
}

#[test]
fn backend_reviews_resume_keep_input_and_clear_at_resolution() {
    use rekindle_types::intent::HeldReview;
    let mut overlay = IntentOverlay::default();
    let review = review(ProofRequired::Acknowledge);
    let id = review.intent_id;
    let held = HeldReview {
        hold_id: id,
        review,
    };
    overlay.replace_held(vec![held.clone()]);
    overlay.handle_key(key(KeyCode::Tab));
    overlay.handle_key(key(KeyCode::Char(' ')));
    overlay.replace_held(vec![held.clone()]);
    assert!(matches!(
        overlay.handle_key(key(KeyCode::Enter)),
        Some(Action::CommitIntent {
            proof: IntentProof::Acknowledged,
            ..
        })
    ));
    overlay.replace_held(vec![held]);
    assert!(overlay.visible());
    overlay.replace_held(Vec::new());
    assert!(!overlay.visible());
}
