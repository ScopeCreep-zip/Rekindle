//! Moderation panel — members with kick/ban/timeout, ban list, pending join queue.

use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::v2::helpers;
use crate::v2::tui::action::{Action, CommandResult};
use crate::v2::tui::focus::{FocusId, FocusRing};
use crate::v2::tui::theme::ThemeManager;
use crate::v2::views::{View, ViewQuery};
use rekindle_types::display::{CommunityDetail, MemberWithPresence};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab { Members, Bans, Queue }

pub struct ModerationView {
    community: String,
    detail: Option<CommunityDetail>,
    bans: Vec<serde_json::Value>,
    pending: Vec<serde_json::Value>,
    tab: Tab,
    selected: usize,
    focus: FocusRing,
    loading: bool,
}

impl ModerationView {
    pub fn new(community: String) -> Self {
        Self {
            community,
            detail: None,
            bans: Vec::new(),
            pending: Vec::new(),
            tab: Tab::Members,
            selected: 0,
            focus: FocusRing::new(vec![FocusId::MessageList]),
            loading: true,
        }
    }

    pub fn community(&self) -> &str { &self.community }

    fn current_list_len(&self) -> usize {
        match self.tab {
            Tab::Members => self.detail.as_ref().map_or(0, |d| d.members.len()),
            Tab::Bans => self.bans.len(),
            Tab::Queue => self.pending.len(),
        }
    }
}

impl ViewQuery for ModerationView {}

impl View for ModerationView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> {
        let title = format!(" Moderation: {} ", self.detail.as_ref().map_or(&self.community as &str, |d| &d.name));
        let block = Block::bordered().title(title).border_style(theme.focused_border());

        if self.loading || self.detail.is_none() {
            frame.render_widget(
                Paragraph::new("  Loading moderation data...").style(theme.style("dim")).block(block),
                area,
            );
            return Ok(());
        }

        let [tab_area, content_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(5),
        ]).areas(area);

        // Tab bar
        let tabs = Line::from(vec![
            Span::styled(if self.tab == Tab::Members { " [Members] " } else { "  Members  " },
                if self.tab == Tab::Members { Style::new().bold().reversed() } else { Style::new().dim() }),
            Span::styled(if self.tab == Tab::Bans { " [Bans] " } else { "  Bans  " },
                if self.tab == Tab::Bans { Style::new().bold().reversed() } else { Style::new().dim() }),
            Span::styled(if self.tab == Tab::Queue { " [Queue] " } else { "  Queue  " },
                if self.tab == Tab::Queue { Style::new().bold().reversed() } else { Style::new().dim() }),
        ]);
        frame.render_widget(Paragraph::new(tabs), tab_area);

        match self.tab {
            Tab::Members => {
                let detail = self.detail.as_ref().unwrap();
                let items: Vec<ListItem<'_>> = detail.members.iter().map(|m| {
                    let line = render_member(m, "    [k]ick [b]an [t]imeout");
                    ListItem::new(line)
                }).collect();
                let mut state = ListState::default();
                state.select(Some(self.selected.min(items.len().saturating_sub(1))));
                frame.render_stateful_widget(
                    List::new(items).block(block).highlight_style(Style::new().reversed()),
                    content_area, &mut state,
                );
            }
            Tab::Bans => {
                let items: Vec<ListItem<'_>> = self.bans.iter().map(|b| {
                    let pseudo = b.get("pseudonymKey").and_then(|v| v.as_str()).unwrap_or("?");
                    let reason = b.get("reason").and_then(|v| v.as_str()).unwrap_or("no reason");
                    let by = b.get("bannedBy").and_then(|v| v.as_str()).unwrap_or("?");
                    ListItem::new(Line::from(vec![
                        Span::raw(format!("  {} ", helpers::abbreviate_key(pseudo))),
                        Span::styled(format!("— banned by {} — \"{reason}\"", helpers::abbreviate_key(by)), Style::new().dim()),
                        Span::styled("    [u]nban", Style::new().dim()),
                    ]))
                }).collect();
                let mut state = ListState::default();
                state.select(Some(self.selected.min(items.len().saturating_sub(1))));
                frame.render_stateful_widget(
                    List::new(items).block(block).highlight_style(Style::new().reversed()),
                    content_area, &mut state,
                );
            }
            Tab::Queue => {
                let items: Vec<ListItem<'_>> = self.pending.iter().map(|p| {
                    let name = p.get("displayName").or_else(|| p.get("display_name")).and_then(|v| v.as_str()).unwrap_or("?");
                    let pseudo = p.get("requesterPseudonymHex").or_else(|| p.get("requester_pseudonym_hex")).and_then(|v| v.as_str()).unwrap_or("?");
                    let has_invite = p.get("inviteCodeHash").or_else(|| p.get("invite_code_hash")).is_some();
                    ListItem::new(Line::from(vec![
                        Span::raw(format!("  → {name} ")),
                        Span::styled(format!("({}) ", helpers::abbreviate_key(pseudo)), Style::new().dim()),
                        Span::styled(if has_invite { "invite: yes" } else { "invite: no" }, Style::new().dim()),
                        Span::styled("    [a]pprove [r]eject", Style::new().dim()),
                    ]))
                }).collect();
                let mut state = ListState::default();
                state.select(Some(self.selected.min(items.len().saturating_sub(1))));
                frame.render_stateful_widget(
                    List::new(items).block(block).highlight_style(Style::new().reversed()),
                    content_area, &mut state,
                );
            }
        }
        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::ScrollDown(_) | Action::Select => {
                if self.selected + 1 < self.current_list_len() { self.selected += 1; }
                Ok(None)
            }
            Action::ScrollUp(_) => {
                self.selected = self.selected.saturating_sub(1);
                Ok(None)
            }
            Action::NextTab => {
                self.tab = match self.tab { Tab::Members => Tab::Bans, Tab::Bans => Tab::Queue, Tab::Queue => Tab::Members };
                self.selected = 0;
                Ok(None)
            }
            Action::PrevTab => {
                self.tab = match self.tab { Tab::Members => Tab::Queue, Tab::Bans => Tab::Members, Tab::Queue => Tab::Bans };
                self.selected = 0;
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<Option<Action>> {
        match result {
            CommandResult::ModerationDataLoaded { detail, bans, pending } => {
                self.detail = Some(detail);
                self.bans = bans;
                self.pending = pending;
                self.loading = false;
            }
            CommandResult::CommunityInfoLoaded { detail } => {
                if detail.governance_key == self.community {
                    self.detail = Some(detail);
                    self.loading = false;
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn on_subscription_event(&mut self, _event: &rekindle_types::subscription_events::SubscriptionEvent) -> Result<Option<Action>> {
        Ok(None)
    }

    fn handle_focused_key(&mut self, key: KeyEvent) -> Option<Action> {
        use crossterm::event::KeyCode;
        let detail = self.detail.as_ref()?;
        match (self.tab, key.code) {
            (Tab::Members, KeyCode::Char('k')) => {
                let member = detail.members.get(self.selected)?;
                Some(Action::KickMember {
                    community: self.community.clone(),
                    pseudonym: member.member.pseudonym_key.clone(),
                    display_name: member.member.display_name.clone(),
                })
            }
            (Tab::Members, KeyCode::Char('b')) => {
                let member = detail.members.get(self.selected)?;
                Some(Action::BanMember {
                    community: self.community.clone(),
                    pseudonym: member.member.pseudonym_key.clone(),
                    display_name: member.member.display_name.clone(),
                })
            }
            (Tab::Members, KeyCode::Char('t')) => {
                let member = detail.members.get(self.selected)?;
                Some(Action::TimeoutMember {
                    community: self.community.clone(),
                    pseudonym: member.member.pseudonym_key.clone(),
                    display_name: member.member.display_name.clone(),
                    duration_secs: 300,
                })
            }
            (Tab::Queue, KeyCode::Char('a')) => {
                let entry = self.pending.get(self.selected)?;
                let pseudo = entry.get("requesterPseudonymHex").or_else(|| entry.get("requester_pseudonym_hex")).and_then(|v| v.as_str())?.to_string();
                let name = entry.get("displayName").or_else(|| entry.get("display_name")).and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                Some(Action::ApproveMember {
                    community: self.community.clone(),
                    pseudonym: pseudo,
                    display_name: name,
                })
            }
            (Tab::Queue, KeyCode::Char('r')) => {
                let entry = self.pending.get(self.selected)?;
                let pseudo = entry.get("requesterPseudonymHex").or_else(|| entry.get("requester_pseudonym_hex")).and_then(|v| v.as_str())?.to_string();
                let name = entry.get("displayName").or_else(|| entry.get("display_name")).and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                Some(Action::RejectMember {
                    community: self.community.clone(),
                    pseudonym: pseudo,
                    display_name: name,
                })
            }
            (Tab::Bans, KeyCode::Char('u')) => {
                let entry = self.bans.get(self.selected)?;
                let pseudo = entry.get("pseudonymKey").and_then(|v| v.as_str())?.to_string();
                Some(Action::UnbanMember {
                    community: self.community.clone(),
                    pseudonym: pseudo,
                })
            }
            _ => None,
        }
    }

    fn focus_ring(&mut self) -> &mut FocusRing { &mut self.focus }
}

/// Render a single member line with status glyph, text label, name, role, timeout, and action hints.
/// Reusable across moderation panel and community_info member list.
fn render_member<'a>(m: &'a MemberWithPresence, action_hints: &'a str) -> Line<'a> {
    let (glyph, label) = match m.status.as_str() {
        "online" => ("●", "[ONLINE]"),
        "away" => ("◐", "[AWAY]"),
        "busy" => ("●", "[BUSY]"),
        "offline" => ("○", "[OFFLINE]"),
        _ => ("○", "[OFFLINE]"),
    };
    let role = m.role_name.as_deref().unwrap_or("");
    let timeout = m.member.timeout_until.map_or(String::new(), |t| format!("  [timeout until {t}]"));
    Line::from(vec![
        Span::raw(format!("  {glyph} {label} ")),
        Span::styled(m.member.display_name.as_str(), Style::new().bold()),
        Span::styled(if role.is_empty() { String::new() } else { format!("  [{role}]") }, Style::new().dim()),
        Span::styled(timeout, Style::new().dim()),
        Span::styled(action_hints, Style::new().dim()),
    ])
}
