//! Activity view (plan E7.15, TU1(a)): the local activity feed as the Host
//! lists it (`ActivityList`, newest first) and announces it
//! (`SystemEvent::ActivityAppended`), the TUI's view of the Infoview's
//! Activity section. The Host derives, stores and names every entry; this
//! view shows each row's time, kind and fields as the DTO carries them.
//!
//! Keys on the list: `j`/`k` move, `o` asks for the page older than the
//! oldest entry shown (`before` = its row id), `r` reads the first page
//! again.

use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState};
use ratatui::Frame;

use rekindle_types::activity::ActivityListItem;
use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

use super::View;
use crate::action::{Action, CommandResult};
use crate::focus::{FocusId, FocusRing};
use crate::theme::ThemeManager;

/// One page of the feed, the CLI's `rekindle activity` default.
pub const ACTIVITY_PAGE: u16 = 20;

/// Activity view state.
pub struct ActivityView {
    focus: FocusRing,
    /// Newest first, in the Host's order.
    entries: Vec<ActivityListItem>,
    list_state: ListState,
}

impl Default for ActivityView {
    fn default() -> Self {
        Self::new()
    }
}

impl ActivityView {
    pub fn new() -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::DmList]),
            entries: Vec::new(),
            list_state: ListState::default(),
        }
    }

    fn step(&mut self, down: bool) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() - 1;
        let next = match self.list_state.selected() {
            None => 0,
            Some(i) if down => (i + 1).min(last),
            Some(i) => i.saturating_sub(1),
        };
        self.list_state.select(Some(next));
    }

    fn holds(&self, id: i64) -> bool {
        self.entries.iter().any(|e| e.id == id)
    }

    /// The first page replaces the list; an older page goes after it.
    fn set_page(&mut self, before: Option<i64>, page: Vec<ActivityListItem>) {
        if before.is_none() {
            self.entries.clear();
        }
        for entry in page {
            if !self.holds(entry.id) {
                self.entries.push(entry);
            }
        }
        self.clamp_selection();
    }

    /// The Host's new entries (newest first) go above the list.
    fn prepend(&mut self, new: &[ActivityListItem]) {
        let fresh: Vec<ActivityListItem> =
            new.iter().filter(|e| !self.holds(e.id)).cloned().collect();
        self.entries.splice(0..0, fresh);
        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        if self.entries.is_empty() {
            self.list_state.select(None);
        } else {
            let at = self.list_state.selected().unwrap_or(0);
            self.list_state.select(Some(at.min(self.entries.len() - 1)));
        }
    }

    /// The request for the page older than the oldest entry shown.
    fn older(&self) -> Action {
        Action::LoadActivity {
            before: self.entries.last().map(|e| e.id),
        }
    }
}

impl View for ActivityView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, _theme: &ThemeManager) -> Result<()> {
        let title = format!(
            " Activity ({}) · [o] older · [r] reload ",
            self.entries.len()
        );
        let items: Vec<ListItem<'static>> = self
            .entries
            .iter()
            .map(|e| ListItem::new(Line::from(Span::raw(rekindle_client::infoview::activity_row(e)))))
            .collect();
        let list = List::new(items)
            .block(
                Block::bordered()
                    .title(title)
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
            Action::Refresh => Some(Action::LoadActivity { before: None }),
            _ => None,
        })
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        if let CommandResult::ActivityLoaded { before, entries } = result {
            self.set_page(before, entries);
        }
        Ok(())
    }

    fn on_subscription_event(&mut self, event: &SubscriptionEvent) -> Result<()> {
        if let SubscriptionEvent::System(SystemEvent::ActivityAppended { entries }) = event {
            self.prepend(entries);
        }
        Ok(())
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.step(true);
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.step(false);
                None
            }
            KeyCode::Char('o') => Some(self.older()),
            _ => None,
        }
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use rekindle_types::activity::ActivityKind;

    fn entry(id: i64, text: &str) -> ActivityListItem {
        ActivityListItem {
            id,
            at: 0,
            kind: ActivityKind::StatusMessage {
                friend: "ab12ab12ab12ab12ab12ab12ab12ab12".into(),
                text: text.into(),
            },
            display_name: Some("Ann".into()),
            nickname: None,
            identity_pin_mismatch: false,
        }
    }

    /// `o` asks for the page before the oldest entry shown, and that page
    /// goes after the list; a reload's first page replaces it.
    #[test]
    fn older_pages_ask_before_the_oldest_entry_and_go_after_it() {
        let mut view = ActivityView::new();
        view.on_command_result(CommandResult::ActivityLoaded {
            before: None,
            entries: vec![entry(9, "newest"), entry(7, "older")],
        })
        .unwrap();
        let Some(Action::LoadActivity { before }) =
            view.handle_focused_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE))
        else {
            panic!("o asks for an older page");
        };
        assert_eq!(before, Some(7));
        view.on_command_result(CommandResult::ActivityLoaded {
            before: Some(7),
            entries: vec![entry(4, "oldest")],
        })
        .unwrap();
        let ids: Vec<i64> = view.entries.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![9, 7, 4]);
        view.on_command_result(CommandResult::ActivityLoaded {
            before: None,
            entries: vec![entry(10, "now")],
        })
        .unwrap();
        assert_eq!(view.entries.len(), 1);
    }
}
