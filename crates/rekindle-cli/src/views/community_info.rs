//! Community info view — metadata, channels, roles, members.
//!
//! Layout:
//! ```text
//! ┌─ dev-team ───────────────────────────────────────────────┐
//! │ Description: Development team coordination               │
//! │ Members: 12          Channels: 3         Created: 2026…  │
//! │ Governance: SMPL     MEK generation: 7                   │
//! ├─ Channels ───────────────────────────────────────────────┤
//! │   #general      text                                     │
//! │   #code-review  text                                     │
//! │   #standup      voice                                    │
//! ├─ Roles ──────────────────────────────────────────────────┤
//! │   owner (1)   admin (2)   member (9)                     │
//! └─────────────────────────────────────────────────────────┘
//! ```

use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use rekindle_types::display::CommunityDetail;

use super::View;
use crate::helpers;
use crate::tui::action::{Action, CommandResult};
use crate::tui::focus::{FocusId, FocusRing};
use crate::tui::theme::ThemeManager;

/// Community info view state.
pub struct CommunityInfoView {
    focus: FocusRing,
    /// Community governance key.
    community: String,
    /// Loaded community detail (None until CommandResult arrives).
    detail: Option<CommunityDetail>,
    /// Whether data is loading.
    loading: bool,
    /// Selected channel index for navigation.
    selected_channel: usize,
}

impl CommunityInfoView {
    /// Create a new community info view.
    pub fn new(community: String) -> Self {
        Self {
            focus: FocusRing::new(vec![FocusId::CommunityInfoPanel]),
            community,
            detail: None,
            loading: true,
            selected_channel: 0,
        }
    }

    /// The community governance key.
    pub fn community(&self) -> &str {
        &self.community
    }

    /// Render the metadata section.
    #[allow(clippy::unused_self)] // Method on self for consistency with View pattern
    fn render_metadata(
        &self,
        frame: &mut Frame,
        area: Rect,
        detail: &CommunityDetail,
        _theme: &ThemeManager,
    ) {
        let gov_short = helpers::abbreviate_key(&detail.governance_key);
        let owner_short = helpers::abbreviate_key(&detail.owner_pseudonym);
        let created = helpers::format_timestamp(detail.created_at);
        let our_key = helpers::abbreviate_key(&detail.our_pseudonym);
        let roles_str = if detail.our_roles.is_empty() {
            "none".to_string()
        } else {
            detail
                .our_roles
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };

        let lines = vec![
            Line::from(vec![
                Span::styled("  Name:         ", Style::new().dim()),
                Span::styled(&detail.name, Style::new().bold()),
            ]),
            if detail.description.is_empty() {
                Line::from("")
            } else {
                Line::from(vec![
                    Span::styled("  Description:  ", Style::new().dim()),
                    Span::raw(&detail.description),
                ])
            },
            Line::from(vec![
                Span::styled("  Members:      ", Style::new().dim()),
                Span::raw(detail.member_count.to_string()),
                Span::styled("      Channels: ", Style::new().dim()),
                Span::raw(detail.channels.len().to_string()),
            ]),
            Line::from(vec![
                Span::styled("  Governance:   ", Style::new().dim()),
                Span::raw(gov_short),
            ]),
            Line::from(vec![
                Span::styled("  Owner:        ", Style::new().dim()),
                Span::raw(owner_short),
                Span::styled("  Created: ", Style::new().dim()),
                Span::raw(created),
            ]),
            Line::from(vec![
                Span::styled("  Your key:     ", Style::new().dim()),
                Span::raw(our_key),
                Span::styled("  Roles: ", Style::new().dim()),
                Span::raw(roles_str),
            ]),
        ];

        let block = Block::bordered()
            .title(format!(" {} ", detail.name))
            .border_style(Style::new());
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }

    /// Render the channels section with selection highlight.
    fn render_channels(&self, frame: &mut Frame, area: Rect, detail: &CommunityDetail) {
        let title = format!(
            " Channels ({}) — j/k navigate, Enter to open ",
            detail.channels.len()
        );
        let block = Block::bordered()
            .title(title)
            .border_style(Style::new().dim());

        if detail.channels.is_empty() {
            let para = Paragraph::new("  No channels.")
                .style(Style::new().dim())
                .block(block);
            frame.render_widget(para, area);
            return;
        }

        let items: Vec<ratatui::widgets::ListItem<'_>> = detail
            .channels
            .iter()
            .enumerate()
            .map(|(i, ch)| {
                let topic = if ch.topic.is_empty() {
                    String::new()
                } else {
                    format!("  — {}", ch.topic)
                };
                let prefix = if i == self.selected_channel {
                    "▸ "
                } else {
                    "  "
                };
                let line = Line::from(vec![
                    Span::raw(format!("{prefix}#{:<20} ", ch.name)),
                    Span::styled(&ch.kind, Style::new().dim()),
                    Span::styled(topic, Style::new().dim()),
                ]);
                ratatui::widgets::ListItem::new(line)
            })
            .collect();

        let mut list_state = ratatui::widgets::ListState::default();
        list_state.select(Some(self.selected_channel));
        let list = ratatui::widgets::List::new(items)
            .block(block)
            .highlight_style(Style::new().reversed());
        frame.render_stateful_widget(list, area, &mut list_state);
    }

    /// Render the roles section.
    #[allow(clippy::unused_self)]
    fn render_roles(&self, frame: &mut Frame, area: Rect, detail: &CommunityDetail) {
        let title = format!(" Roles ({}) ", detail.roles.len());
        let block = Block::bordered()
            .title(title)
            .border_style(Style::new().dim());

        if detail.roles.is_empty() {
            let para = Paragraph::new("  No roles defined.")
                .style(Style::new().dim())
                .block(block);
            frame.render_widget(para, area);
            return;
        }

        let lines: Vec<Line<'_>> = detail
            .roles
            .iter()
            .map(|r| {
                Line::from(vec![
                    Span::raw(format!("  {} ", r.name)),
                    Span::styled(
                        format!(
                            "(id: {}, pos: {}, perms: 0x{:X})",
                            r.id, r.position, r.permissions
                        ),
                        Style::new().dim(),
                    ),
                ])
            })
            .collect();

        frame.render_widget(Paragraph::new(lines).block(block), area);
    }
}

impl View for CommunityInfoView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> {
        if self.loading || self.detail.is_none() {
            let block = Block::bordered()
                .title(format!(
                    " Community: {} ",
                    helpers::abbreviate_key(&self.community)
                ))
                .border_style(theme.focused_border());
            let para = Paragraph::new("  Loading community details...")
                .style(Style::new().dim())
                .block(block);
            frame.render_widget(para, area);
            return Ok(());
        }

        let detail = self.detail.as_ref().expect("detail checked above");

        #[allow(clippy::cast_possible_truncation)] // channels.len() bounded by DHT subkey limit
        let channel_height = (detail.channels.len() as u16 + 2).min(area.height / 3);
        #[allow(clippy::cast_possible_truncation)] // roles.len() bounded by governance manifest
        let role_height = (detail.roles.len() as u16 + 2).min(area.height / 4);

        let [meta_area, channels_area, roles_area] = Layout::vertical([
            Constraint::Length(8),
            Constraint::Length(channel_height),
            Constraint::Min(role_height),
        ])
        .areas(area);

        self.render_metadata(frame, meta_area, detail, theme);
        self.render_channels(frame, channels_area, detail);
        self.render_roles(frame, roles_area, detail);

        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::Refresh => {
                self.loading = true;
                return Ok(Some(Action::ShowCommunityInfo {
                    community: self.community.clone(),
                }));
            }
            Action::ScrollDown(_) => {
                if let Some(ref detail) = self.detail {
                    if !detail.channels.is_empty() {
                        self.selected_channel =
                            (self.selected_channel + 1).min(detail.channels.len() - 1);
                    }
                }
            }
            Action::ScrollUp(_) => {
                self.selected_channel = self.selected_channel.saturating_sub(1);
            }
            Action::Select => {
                // Enter the selected channel
                if let Some(ref detail) = self.detail {
                    if let Some(ch) = detail.channels.get(self.selected_channel) {
                        return Ok(Some(Action::ShowChannel {
                            community: self.community.clone(),
                            channel: ch.name.clone(),
                        }));
                    }
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(ref detail) = self.detail {
                    if !detail.channels.is_empty() {
                        self.selected_channel =
                            (self.selected_channel + 1).min(detail.channels.len() - 1);
                    }
                }
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected_channel = self.selected_channel.saturating_sub(1);
                None
            }
            KeyCode::Enter | KeyCode::Char('l') => {
                if let Some(ref detail) = self.detail {
                    if let Some(ch) = detail.channels.get(self.selected_channel) {
                        return Some(Action::ShowChannel {
                            community: self.community.clone(),
                            channel: ch.name.clone(),
                        });
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        if let CommandResult::CommunityInfoLoaded { detail } = result {
            if detail.governance_key == self.community {
                self.detail = Some(detail);
                self.loading = false;
            }
        }
        Ok(())
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}
