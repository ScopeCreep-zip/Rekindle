//! Invite management — list active invites, create new, revoke existing.
//!
//! All operations dispatch through DaemonRequest and complete via CommandResult.
//! The view renders immediately from cached data and refreshes on GovernanceEvent::InvitesChanged.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
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

pub struct InviteView {
    community: String,
    invites: Vec<serde_json::Value>,
    selected: usize,
    create_mode: bool,
    max_uses_input: String,
    expires_input: String,
    create_field: usize, // 0 = max_uses, 1 = expires
    focus: FocusRing,
    loading: bool,
}

impl InviteView {
    pub fn new(community: String) -> Self {
        Self {
            community,
            invites: Vec::new(),
            selected: 0,
            create_mode: false,
            max_uses_input: "10".into(),
            expires_input: "7d".into(),
            create_field: 0,
            focus: FocusRing::new(vec![FocusId::MessageList]),
            loading: true,
        }
    }

    pub fn community(&self) -> &str { &self.community }

    fn parse_expires(&self) -> Option<u64> {
        let s = self.expires_input.trim();
        if s == "never" || s.is_empty() { return None; }
        let (num_str, unit) = if s.ends_with('d') {
            (&s[..s.len()-1], 86400u64)
        } else if s.ends_with('h') {
            (&s[..s.len()-1], 3600u64)
        } else if s.ends_with('m') {
            (&s[..s.len()-1], 60u64)
        } else {
            (s, 1u64)
        };
        num_str.parse::<u64>().ok().map(|n| n * unit)
    }
}

impl ViewQuery for InviteView {}

impl View for InviteView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> {
        let title = format!(" Invites: {} — [n]ew  [r]evoke  Tab=switch field  Enter=create ", self.community);
        let outer = Block::bordered().title(title).border_style(theme.focused_border());

        if self.loading {
            frame.render_widget(
                Paragraph::new("  Loading invites...").style(theme.style("dim")).block(outer),
                area,
            );
            return Ok(());
        }

        let create_height = if self.create_mode { 3u16 } else { 0 };
        let [list_area, create_area] = Layout::vertical([
            Constraint::Min(5),
            Constraint::Length(create_height),
        ]).areas(area);

        // Invite list
        if self.invites.is_empty() {
            frame.render_widget(
                Paragraph::new("  No active invites. Press [n] to create one.").style(theme.style("dim")).block(outer),
                list_area,
            );
        } else {
            let items: Vec<ListItem<'_>> = self.invites.iter().map(|inv| {
                let code = inv.get("code").or_else(|| inv.get("invite_code"))
                    .and_then(|v| v.as_str()).unwrap_or("?");
                let max = inv.get("maxUses").or_else(|| inv.get("max_uses"))
                    .and_then(|v| v.as_u64());
                let uses = inv.get("useCount").or_else(|| inv.get("use_count"))
                    .and_then(|v| v.as_u64()).unwrap_or(0);
                let uses_str = match max {
                    Some(0) | None => format!("{uses}/unlimited"),
                    Some(m) => format!("{uses}/{m}"),
                };
                let expires = inv.get("expiresAt").or_else(|| inv.get("expires_at"))
                    .and_then(|v| v.as_u64());
                let expires_str = match expires {
                    Some(0) | None => "never".to_string(),
                    Some(ts) => helpers::format_timestamp(ts),
                };
                let code_short = if code.len() > 12 { format!("{}...", &code[..12]) } else { code.to_string() };
                ListItem::new(Line::from(vec![
                    Span::raw(format!("  {code_short:<16} ")),
                    Span::styled(format!("uses: {uses_str:<12} "), Style::new().dim()),
                    Span::styled(format!("expires: {expires_str}"), Style::new().dim()),
                    Span::styled("  [r]evoke", Style::new().dim()),
                ]))
            }).collect();
            let mut state = ListState::default();
            state.select(Some(self.selected.min(items.len().saturating_sub(1))));
            frame.render_stateful_widget(
                List::new(items).block(outer).highlight_style(Style::new().reversed()),
                list_area, &mut state,
            );
        }

        // Create form
        if self.create_mode {
            let max_style = if self.create_field == 0 { Style::new().reversed() } else { Style::new() };
            let exp_style = if self.create_field == 1 { Style::new().reversed() } else { Style::new() };
            let form = Paragraph::new(Line::from(vec![
                Span::raw("  New: max uses "),
                Span::styled(format!("[{}]", self.max_uses_input), max_style),
                Span::raw("  expires "),
                Span::styled(format!("[{}]", self.expires_input), exp_style),
                Span::raw("  → Enter to create, Esc to cancel"),
            ]));
            let form_block = Block::bordered().title(" Create Invite ").border_style(Style::new().dim());
            frame.render_widget(form.block(form_block), create_area);
        }

        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::ScrollDown(_) => {
                if !self.create_mode && self.selected + 1 < self.invites.len() {
                    self.selected += 1;
                }
                Ok(None)
            }
            Action::ScrollUp(_) => {
                if !self.create_mode { self.selected = self.selected.saturating_sub(1); }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<Option<Action>> {
        if let CommandResult::InvitesLoaded { invites } = result {
            self.invites = invites;
            self.loading = false;
        }
        Ok(None)
    }

    fn on_subscription_event(&mut self, event: &rekindle_types::subscription_events::SubscriptionEvent) -> Result<Option<Action>> {
        use rekindle_types::subscription_events::{SubscriptionEvent, GovernanceEvent};
        if let SubscriptionEvent::Governance(GovernanceEvent::InvitesChanged { community }) = event {
            if community == &self.community {
                self.loading = true;
            }
        }
        Ok(None)
    }

    fn handle_focused_key(&mut self, key: KeyEvent) -> Option<Action> {
        if self.create_mode {
            match key.code {
                KeyCode::Tab => {
                    self.create_field = (self.create_field + 1) % 2;
                    return None;
                }
                KeyCode::Char(c) => {
                    match self.create_field {
                        0 => self.max_uses_input.push(c),
                        1 => self.expires_input.push(c),
                        _ => {}
                    }
                    return None;
                }
                KeyCode::Backspace => {
                    match self.create_field {
                        0 => { self.max_uses_input.pop(); }
                        1 => { self.expires_input.pop(); }
                        _ => {}
                    }
                    return None;
                }
                KeyCode::Esc => {
                    self.create_mode = false;
                    return None;
                }
                KeyCode::Enter => {
                    self.create_mode = false;
                    let max_uses = self.max_uses_input.parse::<u32>().unwrap_or(0);
                    let expires = self.parse_expires();
                    return Some(Action::CreateInvite {
                        community: self.community.clone(),
                        max_uses,
                        expires_seconds: expires,
                    });
                }
                _ => return None,
            }
        }

        match key.code {
            KeyCode::Char('n') => {
                self.create_mode = true;
                self.create_field = 0;
                self.max_uses_input = "10".into();
                self.expires_input = "7d".into();
                None
            }
            KeyCode::Char('r') => {
                let inv = self.invites.get(self.selected)?;
                let code = inv.get("code").or_else(|| inv.get("invite_code"))
                    .and_then(|v| v.as_str())?.to_string();
                Some(Action::RevokeInvite {
                    community: self.community.clone(),
                    invite_code: code,
                })
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if self.selected + 1 < self.invites.len() { self.selected += 1; }
                None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            _ => None,
        }
    }

    fn focus_ring(&mut self) -> &mut FocusRing { &mut self.focus }
}
