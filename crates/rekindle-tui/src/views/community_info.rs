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
use rekindle_types::presence::ShareScope;

use super::View;
use crate::action::{Action, CommandResult};
use crate::focus::{FocusId, FocusRing};
use crate::helpers;
use crate::theme::ThemeManager;

/// Community info view state.
pub struct CommunityInfoView {
    focus: FocusRing,
    /// Community anchor key.
    community: String,
    /// Loaded community detail (None until CommandResult arrives).
    detail: Option<CommunityDetail>,
    share_location: Option<ShareScope>,
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
            share_location: None,
            loading: true,
            selected_channel: 0,
        }
    }

    /// The community anchor key.
    pub fn community(&self) -> &str {
        &self.community
    }

    /// Render the metadata section.
    fn render_metadata(&self, frame: &mut Frame, area: Rect, detail: &CommunityDetail) {
        let gov_short = helpers::abbreviate_key(&detail.anchor);
        let owner_short = (detail.creator_pseudonyms.iter())
            .map(|c| helpers::abbreviate_key(c))
            .collect::<Vec<_>>()
            .join(", ");
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
            Line::from(vec![
                Span::styled("  Share location: ", Style::new().dim()),
                Span::raw(match self.share_location {
                    Some(ShareScope::Members) => "On",
                    Some(ShareScope::None) => "Off",
                    None => "Unknown",
                }),
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
    fn render_roles(frame: &mut Frame, area: Rect, detail: &CommunityDetail) {
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

        let channel_count = u16::try_from(detail.channels.len()).unwrap_or(u16::MAX);
        let channel_height = channel_count.saturating_add(2).min(area.height / 3);
        let role_count = u16::try_from(detail.roles.len()).unwrap_or(u16::MAX);
        let role_height = role_count.saturating_add(2).min(area.height / 4);

        let [meta_area, channels_area, roles_area] = Layout::vertical([
            Constraint::Length(9),
            Constraint::Length(channel_height),
            Constraint::Min(role_height),
        ])
        .areas(area);

        self.render_metadata(frame, meta_area, detail);
        self.render_channels(frame, channels_area, detail);
        Self::render_roles(frame, roles_area, detail);

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
            // This community's Infoview (plan E7.15 IV7).
            KeyCode::Char('i') => Some(Action::ShowInfoview {
                subject: rekindle_types::dto::infoview::InfoviewSubject::Community {
                    community: self.community.clone(),
                },
            }),
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
            // Leave this community; the host asks to confirm.
            KeyCode::Char('L') => Some(Action::LeaveCommunity {
                community: self.community.clone(),
            }),
            _ => None,
        }
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        match result {
            CommandResult::CommunityInfoLoaded { detail } if detail.anchor == self.community => {
                self.detail = Some(detail);
                self.loading = false;
            }
            CommandResult::CommunityPresencePolicyLoaded { community, policy }
                if community == self.community =>
            {
                self.share_location = Some(policy.share_location);
            }
            _ => {}
        }
        Ok(())
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}

#[cfg(test)]
#[path = "community_info_tests.rs"]
mod tests;
