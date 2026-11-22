//! Favourite servers view (plan E7.2 "Personal favourite servers"): our
//! `favourite_servers` rows as the Host lists them (`FavouriteServersGet`),
//! with whether an invite may name each one (`invitable`, the Host's call:
//! not a hidden game, current-game sharing on).
//!
//! Keys on the list: `j`/`k` move, `Enter` launches the game to the
//! selected server (`GameLaunch { source: Favourite }`, the Host checks it
//! is ours), `x` removes the selected server (`FavouriteServerRemove`). `i` focuses the input box, where
//! `<game id> <host:port> [label]` adds one or relabels it
//! (`FavouriteServerAdd`); the address is parsed with
//! `ServerAddress::parse` here, so no unparsed address reaches the bus, and
//! the Host checks the rest (the label's 64 characters, the game id).

use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState};
use ratatui::Frame;

use rekindle_types::dto::presence::FavouriteServerView;
use rekindle_types::game_server::{JoinSource, ServerAddress};

use super::View;
use crate::action::{Action, CommandResult, ToastLevel};
use crate::components::input_box::InputBox;
use crate::components::Component;
use crate::focus::{FocusId, FocusRing};
use crate::helpers;
use crate::theme::ThemeManager;

/// Favourite servers view state.
pub struct FavouritesView {
    focus: FocusRing,
    servers: Vec<FavouriteServerView>,
    list_state: ListState,
    input_box: InputBox,
}

impl Default for FavouritesView {
    fn default() -> Self {
        Self::new()
    }
}

impl FavouritesView {
    pub fn new() -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::DmList, FocusId::InputBox]),
            servers: Vec::new(),
            list_state: ListState::default(),
            input_box: InputBox::new(),
        }
    }

    fn step(&mut self, down: bool) {
        if self.servers.is_empty() {
            return;
        }
        let last = self.servers.len() - 1;
        let next = match self.list_state.selected() {
            None => 0,
            Some(i) if down => (i + 1).min(last),
            Some(i) => i.saturating_sub(1),
        };
        self.list_state.select(Some(next));
    }

    fn set_servers(&mut self, servers: Vec<FavouriteServerView>) {
        self.servers = servers;
        if self.servers.is_empty() {
            self.list_state.select(None);
        } else {
            let at = self.list_state.selected().unwrap_or(0);
            self.list_state.select(Some(at.min(self.servers.len() - 1)));
        }
    }

    fn list_items(&self) -> Vec<ListItem<'static>> {
        self.servers
            .iter()
            .map(|s| {
                let (main, note) = row(s);
                ListItem::new(Line::from(vec![
                    Span::raw(main),
                    Span::styled(note, Style::new().add_modifier(Modifier::DIM)),
                ]))
            })
            .collect()
    }

    /// Launch the selected server's game to it.
    fn launch(&self) -> Option<Action> {
        if !self.focus.is_focused(FocusId::DmList) {
            return None;
        }
        let s = self
            .list_state
            .selected()
            .and_then(|i| self.servers.get(i))?;
        Some(match ServerAddress::parse(&s.address) {
            Ok(server) => Action::LaunchGame {
                game_id: s.game_id,
                server: Some(server),
                source: JoinSource::Favourite,
            },
            Err(e) => Action::ShowToast {
                message: format!("{}: {e}", s.address),
                level: ToastLevel::Error,
            },
        })
    }

    /// The input line as an add request, or why it is not one.
    fn parse_add(text: &str) -> Result<Action, String> {
        let mut parts = text.trim().splitn(3, char::is_whitespace);
        let (Some(game), Some(address)) = (parts.next(), parts.next()) else {
            return Err("type <game id> <host:port> [label]".into());
        };
        let game_id = game
            .parse::<u32>()
            .map_err(|_| format!("{game} is not a game id"))?;
        let server = ServerAddress::parse(address).map_err(|e| e.to_string())?;
        let label = parts.next().unwrap_or("").trim().to_string();
        Ok(Action::AddFavourite {
            game_id,
            server,
            label,
        })
    }
}

/// A server's row: its game and address, then its label and whether an
/// invite may name it.
fn row(s: &FavouriteServerView) -> (String, String) {
    let label = if s.label.is_empty() {
        String::new()
    } else {
        format!("  \"{}\"", helpers::sanitize_for_display(&s.label))
    };
    let invite = if s.invitable { "" } else { "  (no invites)" };
    (
        format!(
            "{}  {}",
            helpers::sanitize_for_display(&s.game_name),
            s.address
        ),
        format!("{label}{invite}"),
    )
}

impl View for FavouritesView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, _theme: &ThemeManager) -> Result<()> {
        let [list_area, input_area] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(3)]).areas(area);
        let title = format!(
            " Favourite servers ({}) · [Enter] launch · [x] remove · [i] add: <game id> <host:port> [label] ",
            self.servers.len()
        );
        let list = List::new(self.list_items())
            .block(
                Block::bordered()
                    .title(title)
                    .border_style(super::focus_border(&self.focus, FocusId::DmList)),
            )
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
        frame.render_stateful_widget(list, list_area, &mut self.list_state);
        self.input_box
            .set_focused(self.focus.is_focused(FocusId::InputBox));
        self.input_box.draw(frame, input_area)?;
        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::InputSubmit => {
                let text = self.input_box.content();
                if text.trim().is_empty() {
                    return Ok(None);
                }
                let next = match Self::parse_add(&text) {
                    Ok(add) => {
                        self.input_box.clear();
                        add
                    }
                    Err(message) => Action::ShowToast {
                        message,
                        level: ToastLevel::Warning,
                    },
                };
                return Ok(Some(next));
            }
            Action::ScrollDown(_) => self.step(true),
            Action::ScrollUp(_) => self.step(false),
            Action::Select => return Ok(self.launch()),
            Action::EnterInputMode => self.focus.set(FocusId::InputBox),
            Action::ExitInputMode => self.focus.set(FocusId::DmList),
            _ => {}
        }
        Ok(None)
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        if let CommandResult::FavouritesLoaded { servers } = result {
            self.set_servers(servers);
        }
        Ok(())
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        use crossterm::event::KeyCode;
        if self.focus.is_focused(FocusId::InputBox) {
            return self.input_box.handle_key(key);
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.step(true);
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.step(false);
                None
            }
            KeyCode::Char('x') => {
                let s = self
                    .list_state
                    .selected()
                    .and_then(|i| self.servers.get(i))?;
                let server = ServerAddress::parse(&s.address).ok()?;
                Some(Action::RemoveFavourite {
                    game_id: s.game_id,
                    server,
                })
            }
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

    fn server(address: &str, invitable: bool) -> FavouriteServerView {
        FavouriteServerView {
            game_id: 4181,
            game_name: "Counter-Strike 2".into(),
            icon_key: None,
            address: address.into(),
            label: "scrims".into(),
            added_at: 0,
            invitable,
        }
    }

    #[test]
    fn x_removes_the_selected_server_and_rows_name_uninvitable_ones() {
        let mut view = FavouritesView::new();
        view.on_command_result(CommandResult::FavouritesLoaded {
            servers: vec![server("10.0.0.1:27015", true), server("[::1]:27016", false)],
        })
        .unwrap();
        view.handle_focused_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        let Some(Action::RemoveFavourite { game_id, server }) =
            view.handle_focused_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
        else {
            panic!("x asks to remove the selected server");
        };
        assert_eq!((game_id, server.as_str()), (4181, "[::1]:27016"));
        assert_eq!(
            row(&view.servers[0]),
            (
                "Counter-Strike 2  10.0.0.1:27015".into(),
                "  \"scrims\"".into()
            )
        );
        assert_eq!(row(&view.servers[1]).1, "  \"scrims\"  (no invites)");
    }

    #[test]
    fn enter_launches_the_selected_favourite_to_its_server() {
        let mut view = FavouritesView::new();
        view.on_command_result(CommandResult::FavouritesLoaded {
            servers: vec![server("10.0.0.1:27015", true), server("[::1]:27016", false)],
        })
        .unwrap();
        view.update(Action::ScrollDown(1)).unwrap();
        let Ok(Some(Action::LaunchGame {
            game_id,
            server,
            source,
        })) = view.update(Action::Select)
        else {
            panic!("Enter launches the selected favourite");
        };
        assert_eq!(
            (game_id, server.as_ref().map(ServerAddress::as_str), source),
            (4181, Some("[::1]:27016"), JoinSource::Favourite)
        );
    }

    #[test]
    fn the_input_line_adds_a_parsed_server_or_says_why_not() {
        let Ok(Action::AddFavourite {
            game_id,
            server,
            label,
        }) = FavouritesView::parse_add("4181 10.0.0.1:27015 EU scrims")
        else {
            panic!("a game id, an address and a label add a server");
        };
        assert_eq!(
            (game_id, server.as_str(), label.as_str()),
            (4181, "10.0.0.1:27015", "EU scrims")
        );
        assert!(FavouritesView::parse_add("4181").is_err());
        assert!(FavouritesView::parse_add("cs2 10.0.0.1:27015").is_err());
        assert!(FavouritesView::parse_add("4181 10.0.0.1:27015;rm").is_err());
    }
}
