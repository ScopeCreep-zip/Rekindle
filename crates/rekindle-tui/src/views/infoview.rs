//! Infoview (plan E7.15, IV7; Xfire's Infoview): the Host's view of a
//! subject as it answers `InfoviewGet` and pushes `InfoviewEvent::Updated`
//! to the TUI's window (`TUI_WINDOW`). Its sections, rows and verdicts are
//! the Host's, rendered by `rekindle_client::infoview` as the CLI prints
//! them; this view decides nothing.
//!
//! The TUI does not report a selection to the Host (it never sends
//! `SkinSelectionSet`), so it asks with the explicit subject of what is
//! highlighted: `i` on the dashboard (our own page), on a friend's row, or
//! in a community's info view. Keys on the page: `j`/`k` scroll, `r` asks
//! again.

use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState};
use ratatui::Frame;

use rekindle_client::infoview::{lines, InfoviewLine};
use rekindle_types::dto::infoview::{InfoviewEvent, InfoviewSubject, InfoviewView, TUI_WINDOW};
use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

use super::View;
use crate::action::{Action, CommandResult};
use crate::focus::{FocusId, FocusRing};
use crate::theme::ThemeManager;

/// Infoview state: the subject asked and the Host's lines for it.
pub struct InfoviewPage {
    focus: FocusRing,
    subject: InfoviewSubject,
    lines: Vec<InfoviewLine>,
    list_state: ListState,
}

impl InfoviewPage {
    pub fn new(subject: InfoviewSubject) -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::DmList]),
            subject,
            lines: Vec::new(),
            list_state: ListState::default(),
        }
    }

    /// The subject this page asked for.
    pub fn subject(&self) -> &InfoviewSubject {
        &self.subject
    }

    /// The Host's view replaces the page.
    fn set_view(&mut self, view: &InfoviewView) {
        self.lines = lines(view);
        let at = self
            .list_state
            .selected()
            .map(|i| i.min(self.lines.len().saturating_sub(1)));
        self.list_state.select(if self.lines.is_empty() {
            None
        } else {
            at.or(Some(0))
        });
    }

    fn step(&mut self, down: bool) {
        let last = self.lines.len().saturating_sub(1);
        let at = self.list_state.selected().unwrap_or(0);
        let next = if down {
            (at + 1).min(last)
        } else {
            at.saturating_sub(1)
        };
        if !self.lines.is_empty() {
            self.list_state.select(Some(next));
        }
    }
}

impl View for InfoviewPage {
    fn draw(&mut self, frame: &mut Frame, area: Rect, _theme: &ThemeManager) -> Result<()> {
        let items: Vec<ListItem<'static>> = self
            .lines
            .iter()
            .map(|l| match l {
                InfoviewLine::Heading(h) => ListItem::new(Line::from(Span::styled(
                    h.clone(),
                    Style::new().add_modifier(Modifier::BOLD),
                ))),
                InfoviewLine::Row(r) => ListItem::new(Line::from(Span::raw(format!("  {r}")))),
            })
            .collect();
        let list = List::new(items)
            .block(
                Block::bordered()
                    .title(" Infoview · [r] reload ")
                    .border_style(super::focus_border(&self.focus, FocusId::DmList)),
            )
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
        frame.render_stateful_widget(list, area, &mut self.list_state);
        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        Ok(match action {
            Action::ScrollDown(_) => {
                self.step(true);
                None
            }
            Action::ScrollUp(_) => {
                self.step(false);
                None
            }
            Action::Refresh => Some(Action::ShowInfoview {
                subject: self.subject.clone(),
            }),
            _ => None,
        })
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        if let CommandResult::InfoviewLoaded { view } = result {
            self.set_view(&view);
        }
        Ok(())
    }

    /// The Host's push to the TUI's window replaces the page; a push to
    /// another window (the CLI's, a desktop webview's) is not ours.
    fn on_subscription_event(&mut self, event: &SubscriptionEvent) -> Result<()> {
        if let SubscriptionEvent::System(SystemEvent::Infoview(InfoviewEvent::Updated {
            window,
            view,
        })) = event
        {
            if window == TUI_WINDOW {
                self.set_view(view);
            }
        }
        Ok(())
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.step(true),
            KeyCode::Char('k') | KeyCode::Up => self.step(false),
            _ => {}
        }
        None
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_types::dto::infoview::InfoviewSection;

    fn view(bio: &str) -> InfoviewView {
        InfoviewView {
            subject: InfoviewSubject::Own,
            sections: vec![InfoviewSection::Bio { text: bio.into() }],
        }
    }

    fn pushed(window: &str, bio: &str) -> SubscriptionEvent {
        SubscriptionEvent::System(SystemEvent::Infoview(InfoviewEvent::Updated {
            window: window.into(),
            view: view(bio),
        }))
    }

    /// The Host's answer is the page: each section's heading, then its
    /// rows.
    #[test]
    fn the_answer_renders_the_hosts_sections() {
        let mut page = InfoviewPage::new(InfoviewSubject::Own);
        page.on_command_result(CommandResult::InfoviewLoaded { view: view("hi") })
            .unwrap();
        assert_eq!(
            page.lines,
            vec![
                InfoviewLine::Heading("Bio".into()),
                InfoviewLine::Row("hi".into())
            ]
        );
    }

    /// A push to the TUI's window replaces the page; one to another window
    /// (the CLI's) leaves it.
    #[test]
    fn only_a_push_to_the_tui_window_replaces_the_page() {
        let mut page = InfoviewPage::new(InfoviewSubject::Own);
        page.on_subscription_event(&pushed(TUI_WINDOW, "first"))
            .unwrap();
        page.on_subscription_event(&pushed(rekindle_types::dto::infoview::CLI_WINDOW, "cli"))
            .unwrap();
        assert_eq!(page.lines[1], InfoviewLine::Row("first".into()));
        page.on_subscription_event(&pushed(TUI_WINDOW, "second"))
            .unwrap();
        assert_eq!(page.lines[1], InfoviewLine::Row("second".into()));
    }
}
