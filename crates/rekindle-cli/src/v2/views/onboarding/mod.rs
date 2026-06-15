//! Onboarding wizard — step-by-step community introduction for new members.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::v2::tui::action::{Action, CommandResult, ToastLevel};
use crate::v2::tui::focus::{FocusId, FocusRing};
use crate::v2::tui::theme::ThemeManager;
use crate::v2::views::{View, ViewQuery};
use rekindle_types::dht_types::{OnboardingConfig, WelcomeScreen, QuestionKind};

pub struct OnboardingWizardView {
    community: String,
    config: Option<OnboardingConfig>,
    welcome: Option<WelcomeScreen>,
    current_step: usize,
    answers: std::collections::HashMap<String, String>,
    rules_agreed: bool,
    active_field: usize,
    text_input: String,
    focus: FocusRing,
    loading: bool,
}

impl OnboardingWizardView {
    pub fn new(community: String) -> Self {
        Self {
            community,
            config: None,
            welcome: None,
            current_step: 0,
            answers: std::collections::HashMap::new(),
            rules_agreed: false,
            active_field: 0,
            text_input: String::new(),
            focus: FocusRing::new(vec![FocusId::MessageList]),
            loading: true,
        }
    }

    pub fn community(&self) -> &str { &self.community }

    fn total_steps(&self) -> usize {
        self.config.as_ref().map_or(0, |c| c.steps.len()) + 1 // +1 for welcome/rules
    }

    fn is_welcome_step(&self) -> bool { self.current_step == 0 }

    fn current_questions(&self) -> Option<&[rekindle_types::dht_types::OnboardingQuestion]> {
        if self.current_step == 0 { return None; }
        self.config.as_ref()?.steps.get(self.current_step - 1).map(|s| s.questions.as_slice())
    }
}

impl ViewQuery for OnboardingWizardView {}

impl View for OnboardingWizardView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> {
        let block = Block::bordered()
            .title(format!(" Onboarding: {} — Step {}/{} ", self.community, self.current_step + 1, self.total_steps().max(1)))
            .border_style(theme.focused_border());

        if self.loading {
            frame.render_widget(
                Paragraph::new("  Loading onboarding...").style(theme.style("dim")).block(block),
                area,
            );
            return Ok(());
        }

        let [content_area, nav_area] = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(1),
        ]).areas(area);

        let mut lines: Vec<Line<'_>> = Vec::new();

        if self.is_welcome_step() {
            if let Some(ref welcome) = self.welcome {
                lines.push(Line::from(Span::styled(&welcome.title, Style::new().bold())));
                lines.push(Line::from(""));
                if !welcome.body.is_empty() {
                    lines.push(Line::from(Span::raw(format!("  {}", welcome.body))));
                    lines.push(Line::from(""));
                }
                if !welcome.rules.is_empty() {
                    lines.push(Line::from(Span::styled("  Rules:", Style::new().bold())));
                    for (i, rule) in welcome.rules.iter().enumerate() {
                        lines.push(Line::from(Span::raw(format!("    {}. {rule}", i + 1))));
                    }
                    lines.push(Line::from(""));
                    let checkbox = if self.rules_agreed { "[x]" } else { "[ ]" };
                    lines.push(Line::from(vec![
                        Span::raw(format!("  {checkbox} ")),
                        Span::styled("I agree to the community rules", if self.rules_agreed { Style::new().bold() } else { Style::new() }),
                        Span::styled("  (Space to toggle)", Style::new().dim()),
                    ]));
                }
            } else {
                lines.push(Line::from(Span::raw("  No welcome screen configured.")));
            }
        } else if let Some(questions) = self.current_questions() {
            if let Some(step) = self.config.as_ref().and_then(|c| c.steps.get(self.current_step - 1)) {
                lines.push(Line::from(Span::styled(&step.title, Style::new().bold())));
                if !step.description.is_empty() {
                    lines.push(Line::from(Span::styled(format!("  {}", step.description), Style::new().dim())));
                }
                lines.push(Line::from(""));
            }

            for (i, q) in questions.iter().enumerate() {
                let is_active = i == self.active_field;
                let marker = if is_active { "▸ " } else { "  " };
                let required = if q.required { " *" } else { "" };

                lines.push(Line::from(vec![
                    Span::raw(marker),
                    Span::styled(format!("{}{required}", q.prompt), if is_active { Style::new().bold() } else { Style::new() }),
                ]));

                match &q.kind {
                    QuestionKind::FreeText => {
                        let answer = self.answers.get(&q.id).cloned().unwrap_or_default();
                        let display = if is_active && !self.text_input.is_empty() {
                            self.text_input.clone()
                        } else if answer.is_empty() {
                            "...".to_string()
                        } else {
                            answer
                        };
                        lines.push(Line::from(vec![
                            Span::raw("    ["),
                            Span::styled(display, if is_active { Style::new().reversed() } else { Style::new().dim() }),
                            Span::raw("]"),
                        ]));
                    }
                    QuestionKind::MultipleChoice { options } => {
                        let selected = self.answers.get(&q.id).cloned().unwrap_or_default();
                        for opt in options {
                            let is_selected = selected == *opt;
                            let radio = if is_selected { "(●)" } else { "( )" };
                            lines.push(Line::from(Span::raw(format!("    {radio} {opt}"))));
                        }
                    }
                    QuestionKind::Checkbox => {
                        let checked = self.answers.get(&q.id).map_or(false, |v| v == "true");
                        let cb = if checked { "[x]" } else { "[ ]" };
                        lines.push(Line::from(Span::raw(format!("    {cb}"))));
                    }
                }
                lines.push(Line::from(""));
            }
        }

        frame.render_widget(Paragraph::new(lines).block(block), content_area);

        // Navigation bar
        let can_back = self.current_step > 0;
        let is_last = self.current_step + 1 >= self.total_steps();
        let nav = Line::from(vec![
            Span::styled(if can_back { "  [←/Backspace] Back  " } else { "                      " }, Style::new().dim()),
            Span::styled(
                if is_last { "  [Enter] Submit  " } else { "  [Enter/→] Next  " },
                Style::new().bold(),
            ),
            Span::styled(format!("  Step {}/{}", self.current_step + 1, self.total_steps().max(1)), Style::new().dim()),
        ]);
        frame.render_widget(Paragraph::new(nav), nav_area);

        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::Back => {
                if self.current_step > 0 {
                    self.current_step -= 1;
                    self.active_field = 0;
                    self.text_input.clear();
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<Option<Action>> {
        if let CommandResult::OnboardingLoaded { config, welcome } = result {
            self.config = config;
            self.welcome = welcome;
            self.loading = false;
        }
        Ok(None)
    }

    fn handle_focused_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Enter | KeyCode::Right => {
                if self.is_welcome_step() {
                    let needs_agree = self.config.as_ref().is_some_and(|c| c.require_rules_agreement);
                    let has_rules = self.welcome.as_ref().is_some_and(|w| !w.rules.is_empty());
                    if needs_agree && has_rules && !self.rules_agreed {
                        return Some(Action::ShowToast {
                            message: "Please agree to the community rules first".into(),
                            level: ToastLevel::Warning,
                        });
                    }
                }
                // Save current text input
                if let Some(questions) = self.current_questions() {
                    if let Some(q) = questions.get(self.active_field) {
                        if matches!(q.kind, QuestionKind::FreeText) && !self.text_input.is_empty() {
                            self.answers.insert(q.id.clone(), self.text_input.clone());
                            self.text_input.clear();
                        }
                    }
                }

                if self.current_step + 1 < self.total_steps() {
                    self.current_step += 1;
                    self.active_field = 0;
                    self.text_input.clear();
                } else {
                    // Submit — navigate to channel view
                    return Some(Action::ShowToast {
                        message: "Onboarding complete — welcome!".into(),
                        level: ToastLevel::Success,
                    });
                }
                None
            }
            KeyCode::Left | KeyCode::Backspace if self.text_input.is_empty() => {
                if self.current_step > 0 {
                    self.current_step -= 1;
                    self.active_field = 0;
                }
                None
            }
            KeyCode::Backspace => {
                self.text_input.pop();
                None
            }
            KeyCode::Tab | KeyCode::Down => {
                let max = self.current_questions().map_or(0, |q| q.len());
                if max > 0 { self.active_field = (self.active_field + 1) % max; }
                self.text_input.clear();
                None
            }
            KeyCode::Up => {
                let max = self.current_questions().map_or(0, |q| q.len());
                if max > 0 { self.active_field = if self.active_field == 0 { max - 1 } else { self.active_field - 1 }; }
                self.text_input.clear();
                None
            }
            KeyCode::Char(' ') if self.is_welcome_step() => {
                self.rules_agreed = !self.rules_agreed;
                None
            }
            KeyCode::Char(' ') => {
                if let Some(questions) = self.current_questions() {
                    if let Some(q) = questions.get(self.active_field) {
                        match &q.kind {
                            QuestionKind::Checkbox => {
                                let current = self.answers.get(&q.id).map_or(false, |v| v == "true");
                                self.answers.insert(q.id.clone(), (!current).to_string());
                            }
                            QuestionKind::MultipleChoice { options } => {
                                // Cycle through options
                                let current = self.answers.get(&q.id).cloned().unwrap_or_default();
                                let idx = options.iter().position(|o| *o == current).map_or(0, |i| (i + 1) % options.len());
                                self.answers.insert(q.id.clone(), options[idx].clone());
                            }
                            _ => {}
                        }
                    }
                }
                None
            }
            KeyCode::Char(c) => {
                self.text_input.push(c);
                None
            }
            KeyCode::Esc => Some(Action::Back),
            _ => None,
        }
    }

    fn focus_ring(&mut self) -> &mut FocusRing { &mut self.focus }
}
