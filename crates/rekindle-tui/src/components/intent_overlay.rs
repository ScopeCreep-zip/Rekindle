//! The intent review overlay (plan D3.4, U1.7; ADR 0019): fed by the host's
//! `IntentRequired` answers and `IntentPending` events, it renders the
//! review and collects the proof its class asks for. The TUI never decides
//! whether to ask.
//!
//! Four proof widgets: a verb + object button (Commit), a checkbox stating
//! the trust fact (Acknowledge), a field for the subject's name
//! (TypeSubject) and a masked field (Passphrase). Esc cancels, except a
//! review whose cancel sends something (`cancel_sends`, plan F6 U-2): only
//! Enter on Cancel resolves that one. Cancel has
//! the focus first; colour is never the only signal (`[Cancel]`/`[Commit]`
//! labels, `[x]` for the checkbox).
//!
//! Source patterns:
//! - oxicord `presentation/widgets/confirmation_modal.rs` — Clear + centered_rect
//! - vortix `ui/overlays/confirm_dialog.rs` — generic ConfirmDialogConfig

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;
use rekindle_types::intent::{IntentProof, IntentReview, ProofRequired};
use zeroize::Zeroizing;

use crate::action::Action;
use crate::app::centered_rect;
use crate::theme::ThemeManager;

/// The overlay's state: the review on screen, if any, and its proof input.
#[derive(Default)]
pub struct IntentOverlay {
    review: Option<IntentReview>,
    /// Commit has the focus (Cancel first, the safe default).
    commit_focused: bool,
    acknowledged: bool,
    typed: String,
    passphrase: Zeroizing<String>,
}

impl IntentOverlay {
    /// Show `review`, replacing any other.
    pub fn show(&mut self, review: IntentReview) {
        *self = Self {
            review: Some(review),
            ..Self::default()
        };
    }

    #[must_use]
    pub fn visible(&self) -> bool {
        self.review.is_some()
    }

    /// Handle a key while visible: the commit or the cancel it resolves to.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        let proof = self.review.as_ref()?.proof;
        match key.code {
            KeyCode::Tab | KeyCode::Left | KeyCode::Right => {
                self.commit_focused = !self.commit_focused;
            }
            KeyCode::Enter if self.commit_focused => return self.commit(),
            // A cancel that sends (a decline) is never a stray Esc.
            KeyCode::Esc if self.review.as_ref().is_some_and(|r| r.cancel_sends) => {}
            // Esc, or Enter on Cancel.
            KeyCode::Esc | KeyCode::Enter => return self.cancel(),
            KeyCode::Char(' ') if proof == ProofRequired::Acknowledge => {
                self.acknowledged = !self.acknowledged;
            }
            KeyCode::Backspace => match proof {
                ProofRequired::TypeSubject => {
                    self.typed.pop();
                }
                ProofRequired::Passphrase => {
                    self.passphrase.pop();
                }
                ProofRequired::None | ProofRequired::Commit | ProofRequired::Acknowledge => {}
            },
            KeyCode::Char(c) => match proof {
                ProofRequired::TypeSubject => self.typed.push(c),
                ProofRequired::Passphrase => self.passphrase.push(c),
                ProofRequired::None | ProofRequired::Commit | ProofRequired::Acknowledge => {}
            },
            _ => {}
        }
        None
    }

    fn cancel(&mut self) -> Option<Action> {
        let review = self.review.take()?;
        Some(Action::CancelIntent {
            intent_id: review.intent_id,
        })
    }

    /// The commit, when the proof is complete; otherwise nothing happens.
    fn commit(&mut self) -> Option<Action> {
        let review = self.review.as_ref()?;
        let proof = match review.proof {
            ProofRequired::None | ProofRequired::Commit => IntentProof::Commit,
            ProofRequired::Acknowledge if self.acknowledged => IntentProof::Acknowledged,
            ProofRequired::TypeSubject if !self.typed.is_empty() => {
                IntentProof::Typed(self.typed.clone())
            }
            ProofRequired::Passphrase if !self.passphrase.is_empty() => {
                IntentProof::Passphrase(self.passphrase.clone())
            }
            ProofRequired::Acknowledge | ProofRequired::TypeSubject | ProofRequired::Passphrase => {
                return None
            }
        };
        let review = self.review.take()?;
        Some(Action::CommitIntent {
            review: Box::new(review),
            proof,
        })
    }
}

/// Render the overlay centered over `area`.
pub fn render(frame: &mut Frame, area: Rect, state: &IntentOverlay, theme: &ThemeManager) {
    let Some(review) = &state.review else {
        return;
    };
    let popup = centered_rect(
        area,
        64u16.min(area.width.saturating_sub(4)),
        14u16.min(area.height.saturating_sub(2)),
    );
    frame.render_widget(Clear, popup);
    let block = Block::bordered()
        .title(format!(" Confirm: {:?} ", review.class))
        .border_style(Style::default().fg(theme.color("warning")));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let [text_area, proof_area, button_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    let mut lines = vec![Line::from(format!(
        "  {:?} {}",
        review.kind,
        review.subject.typed_name()
    ))];
    lines.extend(
        review
            .facts
            .iter()
            .map(|fact| Line::from(Span::styled(format!("  - {fact:?}"), Style::new().dim()))),
    );
    lines.extend(
        review
            .escalations
            .iter()
            .map(|e| Line::from(Span::styled(format!("  ! {e:?}"), Style::new().bold()))),
    );
    lines.push(Line::from(Span::styled(
        format!(
            "  reversibility: {:?}; notifies: {:?}",
            review.reversibility, review.notifies
        ),
        Style::new().dim(),
    )));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), text_area);

    let proof_line = match review.proof {
        ProofRequired::None | ProofRequired::Commit => String::new(),
        ProofRequired::Acknowledge => format!(
            "  [{}] I understand (space)",
            if state.acknowledged { "x" } else { " " }
        ),
        ProofRequired::TypeSubject => format!(
            "  Type \"{}\": {}",
            review.subject.typed_name(),
            state.typed
        ),
        ProofRequired::Passphrase => {
            format!(
                "  Passphrase: {}",
                "*".repeat(state.passphrase.chars().count())
            )
        }
    };
    frame.render_widget(Paragraph::new(proof_line), proof_area);

    let focused = Style::default()
        .fg(theme.color("bg.base"))
        .bg(theme.color("accent.primary"))
        .bold();
    let (cancel_style, commit_style) = if state.commit_focused {
        (Style::new().dim(), focused.bg(theme.color("error")))
    } else {
        (focused, Style::new().dim())
    };
    let buttons = Line::from(vec![
        Span::styled(" [Cancel] ", cancel_style),
        Span::raw("   "),
        Span::styled(" [Commit] ", commit_style),
    ]);
    frame.render_widget(
        Paragraph::new(buttons).alignment(Alignment::Center),
        button_area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use rekindle_types::intent::{
        IntentClass, IntentId, IntentKind, Notifies, Reversibility, Subject,
    };

    fn review(proof: ProofRequired) -> IntentReview {
        IntentReview {
            intent_id: IntentId::new(),
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
}
