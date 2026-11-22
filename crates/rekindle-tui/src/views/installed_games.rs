//! Installed games view (plan E7.15 IV3): the games we can launch, as the
//! Host lists them (`InstalledGamesGet`), each with whether `GameLaunch`
//! without a server can start it (`launchable`, the Host's call).
//!
//! Keys: `j`/`k` move, `Enter` launches the selected game
//! (`GameLaunch { source: Installed }`). A game the Host cannot launch is
//! still listed, its launch disabled with the reason.

use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState};
use ratatui::Frame;

use rekindle_types::dto::presence::InstalledGameView;
use rekindle_types::game_server::JoinSource;

use super::View;
use crate::action::{Action, CommandResult, ToastLevel};
use crate::focus::{FocusId, FocusRing};
use crate::helpers;
use crate::theme::ThemeManager;

/// Why a game's launch is disabled.
pub(crate) const CANNOT_LAUNCH: &str = "can't be launched here";

/// Installed games view state.
pub struct InstalledGamesView {
    focus: FocusRing,
    games: Vec<InstalledGameView>,
    list_state: ListState,
}

impl Default for InstalledGamesView {
    fn default() -> Self {
        Self::new()
    }
}

impl InstalledGamesView {
    pub fn new() -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::DmList]),
            games: Vec::new(),
            list_state: ListState::default(),
        }
    }

    fn step(&mut self, down: bool) {
        if self.games.is_empty() {
            return;
        }
        let last = self.games.len() - 1;
        let next = match self.list_state.selected() {
            None => 0,
            Some(i) if down => (i + 1).min(last),
            Some(i) => i.saturating_sub(1),
        };
        self.list_state.select(Some(next));
    }

    fn set_games(&mut self, games: Vec<InstalledGameView>) {
        self.games = games;
        if self.games.is_empty() {
            self.list_state.select(None);
        } else {
            let at = self.list_state.selected().unwrap_or(0);
            self.list_state.select(Some(at.min(self.games.len() - 1)));
        }
    }

    /// Launch the selected game, or say why it cannot be.
    fn launch(&self) -> Option<Action> {
        let game = self.list_state.selected().and_then(|i| self.games.get(i))?;
        Some(if game.launchable {
            Action::LaunchGame {
                game_id: game.game_id,
                server: None,
                source: JoinSource::Installed,
            }
        } else {
            Action::ShowToast {
                message: format!(
                    "{} {CANNOT_LAUNCH}",
                    helpers::sanitize_for_display(&game.game_name)
                ),
                level: ToastLevel::Warning,
            }
        })
    }
}

/// A game's row: its name, then why its launch is disabled, if it is.
fn row(g: &InstalledGameView) -> (String, String) {
    let note = if g.launchable {
        String::new()
    } else {
        format!("  ({CANNOT_LAUNCH})")
    };
    (helpers::sanitize_for_display(&g.game_name), note)
}

impl View for InstalledGamesView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, _theme: &ThemeManager) -> Result<()> {
        let title = format!(" Installed games ({}) · [Enter] launch ", self.games.len());
        let items: Vec<ListItem<'static>> = self
            .games
            .iter()
            .map(|g| {
                let (name, note) = row(g);
                ListItem::new(Line::from(vec![
                    Span::raw(name),
                    Span::styled(note, Style::new().add_modifier(Modifier::DIM)),
                ]))
            })
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
        match action {
            Action::ScrollDown(_) => self.step(true),
            Action::ScrollUp(_) => self.step(false),
            Action::Select => return Ok(self.launch()),
            _ => {}
        }
        Ok(None)
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        if let CommandResult::InstalledGamesLoaded { games } = result {
            self.set_games(games);
        }
        Ok(())
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(game_id: u32, name: &str, launchable: bool) -> InstalledGameView {
        InstalledGameView {
            game_id,
            game_name: name.into(),
            icon_key: None,
            launchable,
        }
    }

    fn loaded() -> InstalledGamesView {
        let mut view = InstalledGamesView::new();
        view.on_command_result(CommandResult::InstalledGamesLoaded {
            games: vec![
                game(4181, "Counter-Strike 2", true),
                game(8507, "Battlefield 2", false),
            ],
        })
        .unwrap();
        view
    }

    #[test]
    fn enter_launches_the_selected_installed_game_without_a_server() {
        let mut view = loaded();
        let Ok(Some(Action::LaunchGame {
            game_id,
            server,
            source,
        })) = view.update(Action::Select)
        else {
            panic!("Enter launches a launchable game");
        };
        assert_eq!(
            (game_id, server, source),
            (4181, None, JoinSource::Installed)
        );
    }

    #[test]
    fn a_game_that_cannot_launch_is_listed_and_says_why() {
        let mut view = loaded();
        view.update(Action::ScrollDown(1)).unwrap();
        assert_eq!(
            row(&view.games[1]),
            ("Battlefield 2".into(), "  (can't be launched here)".into())
        );
        let Ok(Some(Action::ShowToast { message, .. })) = view.update(Action::Select) else {
            panic!("a disabled launch shows the reason and sends nothing");
        };
        assert_eq!(message, "Battlefield 2 can't be launched here");
    }
}
