//! Servers view (plan E7.2 G3, SK2-G design §3; Xfire's Servers tab): the
//! Host's `ServersView` — favourites, recent visits, our friends' servers,
//! then one list per community — as it answers `ServersView` and pushes
//! `SystemEvent::ServersViewChanged`. Every row shows its Join, Add to
//! favourites and Remove with the Host's verdict; a refused action is shown
//! disabled with its reason and, pressed, says why instead of asking the
//! Host (memory: backend owns policy). This view decides nothing.
//!
//! Keys on the list: `j`/`k` move between rows, `Enter` joins
//! (`GameLaunch` with the row's launch source), `f` adds the row to our
//! favourites (`FavouriteServerAdd`), `x` removes it
//! (`FavouriteServerRemove` for a favourite, `RecentServerRemove` for a
//! recent visit), `r` reads the view again.

use std::fmt::Write as _;

use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState};
use ratatui::Frame;

use rekindle_types::dto::servers::{
    Joinability, ServerCategory, ServerCategoryKind, ServerRefusal, ServerRow, ServerVerdict,
    ServersView as HostServersView,
};
use rekindle_types::friend::NotJoinableReason;
use rekindle_types::game_server::{JoinSource, ServerAddress};
use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

use super::View;
use crate::action::{Action, CommandResult, ToastLevel};
use crate::focus::{FocusId, FocusRing};
use crate::helpers;
use crate::theme::ThemeManager;

/// One line of the list: a category's header or one of its rows.
enum Item {
    Header(String),
    Row(ServerRow),
}

/// Servers view state.
pub struct ServersView {
    focus: FocusRing,
    items: Vec<Item>,
    list_state: ListState,
}

impl Default for ServersView {
    fn default() -> Self {
        Self::new()
    }
}

impl ServersView {
    pub fn new() -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::DmList]),
            items: Vec::new(),
            list_state: ListState::default(),
        }
    }

    /// The Host's view replaces the list; the selection stays on the same
    /// line where it still is a row.
    fn set_view(&mut self, view: HostServersView) {
        self.items = view
            .categories
            .into_iter()
            .flat_map(|c| {
                let header = Item::Header(header(&c));
                std::iter::once(header).chain(c.rows.into_iter().map(Item::Row))
            })
            .collect();
        let at = self.list_state.selected().unwrap_or(0);
        let row = (at..self.items.len())
            .chain((0..at).rev())
            .find(|&i| matches!(self.items[i], Item::Row(_)));
        self.list_state.select(row);
    }

    fn rows(&self) -> usize {
        self.items
            .iter()
            .filter(|i| matches!(i, Item::Row(_)))
            .count()
    }

    /// Move to the next or previous row, skipping headers.
    fn step(&mut self, down: bool) {
        let at = self.list_state.selected();
        let next = if down {
            let from = at.map_or(0, |i| i + 1);
            (from..self.items.len()).find(|&i| matches!(self.items[i], Item::Row(_)))
        } else {
            let to = at.unwrap_or(0);
            (0..to)
                .rev()
                .find(|&i| matches!(self.items[i], Item::Row(_)))
        };
        if let Some(next) = next.or(at) {
            self.list_state.select(Some(next));
        }
    }

    fn selected(&self) -> Option<&ServerRow> {
        match self.list_state.selected().and_then(|i| self.items.get(i)) {
            Some(Item::Row(row)) => Some(row),
            _ => None,
        }
    }

    /// The selected row's Join, or why it is disabled.
    fn join(&self) -> Option<Action> {
        let row = self.selected()?;
        if let Joinability::NotJoinable { reason } = row.join.joinability {
            return Some(disabled("Join", not_joinable(reason)));
        }
        let Some(game_id) = row.join.game_id else {
            return Some(disabled("Join", refusal(ServerRefusal::NoNumericGame)));
        };
        let server = match row.join.server.as_deref().map(ServerAddress::parse) {
            None => None,
            Some(Ok(server)) => Some(server),
            Some(Err(_)) => return Some(disabled("Join", refusal(ServerRefusal::BadAddress))),
        };
        Some(Action::LaunchGame {
            game_id,
            server,
            source: row.join.launch_source,
        })
    }

    /// The selected row's Add to favourites, or why it is disabled.
    fn add_favourite(&self) -> Option<Action> {
        let row = self.selected()?;
        if let ServerVerdict::Refused { reason } = row.add_favourite {
            return Some(disabled("Add to favourites", refusal(reason)));
        }
        let (Some(game_id), Ok(server)) = (row.game_id, ServerAddress::parse(&row.address)) else {
            return Some(disabled(
                "Add to favourites",
                refusal(ServerRefusal::BadAddress),
            ));
        };
        Some(Action::AddFavourite {
            game_id,
            server,
            label: row.label.clone(),
        })
    }

    /// The selected row's Remove, or why it is disabled.
    fn remove(&self) -> Option<Action> {
        let row = self.selected()?;
        if let ServerVerdict::Refused { reason } = row.remove {
            return Some(disabled("Remove", refusal(reason)));
        }
        let (Some(game_id), Ok(server)) = (row.game_id, ServerAddress::parse(&row.address)) else {
            return Some(disabled("Remove", refusal(ServerRefusal::BadAddress)));
        };
        Some(match row.source {
            JoinSource::Recent => Action::RemoveRecentServer { game_id, server },
            JoinSource::Favourite => Action::RemoveFavourite { game_id, server },
            _ => disabled("Remove", refusal(ServerRefusal::NotRemovable)),
        })
    }
}

/// A disabled action, pressed: the Host's reason, never a request.
fn disabled(action: &str, reason: &str) -> Action {
    Action::ShowToast {
        message: format!("{action}: {reason}"),
        level: ToastLevel::Warning,
    }
}

fn not_joinable(reason: NotJoinableReason) -> &'static str {
    match reason {
        NotJoinableReason::NoServer => "not on a server",
        NotJoinableReason::CannotLaunch => "Rekindle cannot start this game on a server",
    }
}

fn refusal(reason: ServerRefusal) -> &'static str {
    match reason {
        ServerRefusal::AlreadyFavourite => "already a favourite",
        ServerRefusal::NoNumericGame => "not a game Rekindle knows",
        ServerRefusal::BadAddress => "not a host:port address",
        ServerRefusal::NotRemovable => "only a favourite or a recent server can be removed",
    }
}

fn header(c: &ServerCategory) -> String {
    let name = match (c.kind, &c.community) {
        (ServerCategoryKind::Favourites, _) => "Favourites".to_string(),
        (ServerCategoryKind::Recent, _) => "Recent".to_string(),
        (ServerCategoryKind::Friends, _) => "Friends".to_string(),
        (ServerCategoryKind::Community, Some(c)) => helpers::sanitize_for_display(&c.name),
        (ServerCategoryKind::Community, None) => "Community".to_string(),
    };
    format!("{name} ({})", c.rows.len())
}

/// An action's label: plain when allowed, `label: reason` when refused.
fn verdict(label: &str, refused: Option<&str>) -> Span<'static> {
    match refused {
        None => Span::raw(format!("  [{label}]")),
        Some(reason) => Span::styled(
            format!("  [{label}: {reason}]"),
            Style::new().add_modifier(Modifier::DIM),
        ),
    }
}

fn refused(v: ServerVerdict) -> Option<&'static str> {
    match v {
        ServerVerdict::Allowed => None,
        ServerVerdict::Refused { reason } => Some(refusal(reason)),
    }
}

/// A row: its game, address, label and friend, then its three actions with
/// their verdicts.
pub(crate) fn row_line(r: &ServerRow) -> Line<'static> {
    let mut main = format!(
        "  {}  {}",
        helpers::sanitize_for_display(&r.game_name),
        helpers::sanitize_for_display(&r.address)
    );
    if !r.label.is_empty() {
        let _ = write!(main, "  \"{}\"", helpers::sanitize_for_display(&r.label));
    }
    if let Some(friend) = &r.friend {
        let _ = write!(
            main,
            "  {} ({})",
            helpers::sanitize_for_display(&friend.display_name),
            helpers::abbreviate_key(&friend.public_key)
        );
    }
    let join = match r.join.joinability {
        Joinability::Joinable => None,
        Joinability::NotJoinable { reason } => Some(not_joinable(reason)),
    };
    Line::from(vec![
        Span::raw(main),
        verdict("Join", join),
        verdict("Add to favourites", refused(r.add_favourite)),
        verdict("Remove", refused(r.remove)),
    ])
}

impl View for ServersView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, _theme: &ThemeManager) -> Result<()> {
        let title = format!(
            " Servers ({}) · [Enter] join · [f] add to favourites · [x] remove · [r] reload ",
            self.rows()
        );
        let items: Vec<ListItem<'static>> = self
            .items
            .iter()
            .map(|i| match i {
                Item::Header(h) => ListItem::new(Line::from(Span::styled(
                    h.clone(),
                    Style::new().add_modifier(Modifier::BOLD),
                ))),
                Item::Row(r) => ListItem::new(row_line(r)),
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
        Ok(match action {
            Action::ScrollDown(_) => {
                self.step(true);
                None
            }
            Action::ScrollUp(_) => {
                self.step(false);
                None
            }
            Action::Select => self.join(),
            Action::Refresh => Some(Action::ShowServers),
            _ => None,
        })
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        if let CommandResult::ServersViewLoaded { view } = result {
            self.set_view(view);
        }
        Ok(())
    }

    fn on_subscription_event(&mut self, event: &SubscriptionEvent) -> Result<()> {
        if let SubscriptionEvent::System(SystemEvent::ServersViewChanged { view }) = event {
            self.set_view(view.clone());
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
            KeyCode::Char('f') => self.add_favourite(),
            KeyCode::Char('x') => self.remove(),
            _ => None,
        }
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}

#[cfg(test)]
#[path = "servers_tests.rs"]
mod tests;
