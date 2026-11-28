//! Profile view (plan E7.2: "The TUI's profile view shows bio and
//! totals"): a friend's bio as their last `ProfileBio` stated it and the
//! game time they last shared (`FriendBioGet`, `FriendGameTotalsGet`), or,
//! opened from the dashboard, our own (`ProfileBioGet`, `GameTotalsGet`).
//!
//! Layout:
//! ```text
//! ┌─ Profile: alice ──────────────────────────────────────────┐
//! │ Bio                                                       │
//! │   Sniper main. EU evenings.                               │
//! │ Game time                                                 │
//! │   Counter-Strike 2        2h 30m   last played 2m ago     │
//! └──────────────────────────────────────────────────────────┘
//! ```
//!
//! The bio is the peer's own text: it is shown sanitized, never
//! interpreted.

use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::Frame;

use rekindle_types::dto::presence::GameTotalView;

use super::View;
use crate::action::{Action, CommandResult};
use crate::focus::{FocusId, FocusRing};
use crate::helpers;
use crate::theme::ThemeManager;

/// What the Host answered for the profile.
struct Loaded {
    bio: Option<String>,
    totals: Vec<GameTotalView>,
}

/// One profile: a friend's (`peer_key`) or our own (`None`).
pub struct ProfileView {
    focus: FocusRing,
    peer_key: Option<String>,
    /// The name the opener knew, shown in the title.
    name: String,
    loaded: Option<Loaded>,
}

impl ProfileView {
    pub fn new(peer_key: Option<String>, name: String) -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::MessageList]),
            peer_key,
            name,
            loaded: None,
        }
    }

    /// Whose profile this is.
    pub fn peer_key(&self) -> Option<&str> {
        self.peer_key.as_deref()
    }

    fn lines(&self) -> Vec<Line<'static>> {
        let heading = |text: &str| {
            Line::from(Span::styled(
                text.to_string(),
                Style::new().add_modifier(Modifier::BOLD),
            ))
        };
        let Some(loaded) = &self.loaded else {
            return vec![Line::raw("Loading…")];
        };
        let mut lines = vec![heading("Bio")];
        lines.push(Line::raw(match &loaded.bio {
            Some(bio) => format!("  {}", helpers::sanitize_for_display(bio)),
            None => "  No bio".into(),
        }));
        lines.push(Line::raw(""));
        lines.push(heading("Game time"));
        if loaded.totals.is_empty() {
            lines.push(Line::raw(if self.peer_key.is_some() {
                "  Nothing shared"
            } else {
                "  No games recorded"
            }));
        }
        let now = now_ms();
        for game in &loaded.totals {
            let ago = u64::try_from(now.saturating_sub(game.last_played)).unwrap_or(0);
            lines.push(Line::from(vec![
                Span::raw(format!(
                    "  {:<28} {:>8}",
                    helpers::sanitize_for_display(&game.game_name),
                    helpers::format_uptime(game.total_secs),
                )),
                Span::styled(
                    format!(
                        "   last played {}",
                        helpers::format_duration_ago(std::time::Duration::from_millis(ago))
                    ),
                    Style::new().add_modifier(Modifier::DIM),
                ),
            ]));
        }
        lines
    }
}

/// The wall clock in Unix ms, as `GameTotalView::last_played` counts.
fn now_ms() -> i64 {
    i64::try_from(rekindle_utils::timestamp_ms()).unwrap_or(i64::MAX)
}

impl View for ProfileView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, _theme: &ThemeManager) -> Result<()> {
        let title = if self.peer_key.is_some() {
            format!(" Profile: {} ", helpers::sanitize_for_display(&self.name))
        } else {
            " Your profile ".into()
        };
        let body = Paragraph::new(self.lines())
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(title));
        frame.render_widget(body, area);
        Ok(())
    }

    fn update(&mut self, _action: Action) -> Result<Option<Action>> {
        Ok(None)
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        if let CommandResult::ProfileLoaded {
            peer_key,
            bio,
            totals,
        } = result
        {
            if peer_key == self.peer_key {
                self.loaded = Some(Loaded { bio, totals });
            }
        }
        Ok(())
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
