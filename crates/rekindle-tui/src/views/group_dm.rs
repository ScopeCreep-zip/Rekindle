//! Group DMs view (plan E2.6): our groups and the invites we have not
//! answered on the left, the selected group's stored messages on the
//! right.
//!
//! Keys on the list: `j`/`k` move, `Enter` opens a group's messages, `a`
//! accepts and `x` declines a pending invite, `L` leaves a group. The
//! host reviews each of those (its intent gate); this view only asks.
//!
//! Under the open group's messages, the input box sends
//! (`IpcRequest::GroupDmSend`); a group message, ours or a member's,
//! reloads the open group's messages.

use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use rekindle_types::disappearing::{notice_text, remaining_short};
use rekindle_types::dto::dm::DmMessage;
use rekindle_types::dto::group_dm::GroupDmView;
use rekindle_types::message::{DeliveryRetry, SendState};
use rekindle_types::subscription_events::{
    ChannelMessageEvent, ConversationContext, GroupDmEvent, SubscriptionEvent,
};

use super::View;
use crate::action::{Action, CommandResult};
use crate::components::input_box::InputBox;
use crate::components::Component;
use crate::focus::{FocusId, FocusRing};
use crate::helpers;
use crate::theme::ThemeManager;

/// Group DMs view state.
pub struct GroupDmListView {
    focus: FocusRing,
    groups: Vec<GroupDmView>,
    list_state: ListState,
    /// The group whose messages are shown, and those messages (with the
    /// stored disappearing-timer notices, plan E7.1).
    open: Option<(String, Vec<DmMessage>)>,
    input_box: InputBox,
    /// Our open group's sends the Host is retrying, by client id (plan
    /// SR2b): set on an unanswered attempt, gone once the row settles.
    retries: std::collections::HashMap<String, DeliveryRetry>,
}

impl Default for GroupDmListView {
    fn default() -> Self {
        Self::new()
    }
}

impl GroupDmListView {
    pub fn new() -> Self {
        Self {
            focus: FocusRing::new(vec![
                FocusId::DmList,
                FocusId::MessageList,
                FocusId::InputBox,
            ]),
            groups: Vec::new(),
            list_state: ListState::default(),
            open: None,
            input_box: InputBox::new(),
            retries: std::collections::HashMap::new(),
        }
    }

    fn selected(&self) -> Option<&GroupDmView> {
        self.list_state.selected().and_then(|i| self.groups.get(i))
    }

    fn step(&mut self, down: bool) {
        if self.groups.is_empty() {
            return;
        }
        let last = self.groups.len() - 1;
        let next = match self.list_state.selected() {
            None => 0,
            Some(i) if down => (i + 1).min(last),
            Some(i) => i.saturating_sub(1),
        };
        self.list_state.select(Some(next));
    }

    fn set_groups(&mut self, groups: Vec<GroupDmView>) {
        self.groups = groups;
        if self.groups.is_empty() {
            self.list_state.select(None);
        } else {
            let at = self.list_state.selected().unwrap_or(0);
            self.list_state.select(Some(at.min(self.groups.len() - 1)));
        }
        let still_held = |id: &str| self.groups.iter().any(|g| g.group_id == id && !g.pending);
        if self.open.as_ref().is_some_and(|(id, _)| !still_held(id)) {
            self.open = None;
        }
    }

    /// A member's label in the open group, or their abbreviated key.
    fn label(&self, group_id: &str, key: &str) -> String {
        self.groups
            .iter()
            .find(|g| g.group_id == group_id)
            .and_then(|g| g.members.iter().find(|m| m.key == key))
            .map_or_else(|| helpers::abbreviate_key(key), |m| m.label.clone())
    }

    fn list_items(&self) -> Vec<ListItem<'static>> {
        self.groups
            .iter()
            .map(|g| {
                let name = helpers::sanitize_for_display(&g.name);
                let tag = if g.pending { " (invite)" } else { "" };
                ListItem::new(Line::from(vec![
                    Span::raw(name),
                    Span::styled(
                        format!("{tag} · {}", g.members.len()),
                        Style::new().add_modifier(Modifier::DIM),
                    ),
                ]))
            })
            .collect()
    }

    fn message_lines(&self) -> (String, Vec<Line<'static>>) {
        let Some((group_id, messages)) = &self.open else {
            return (" Messages ".into(), vec![Line::raw("Enter opens a group.")]);
        };
        let title = self
            .groups
            .iter()
            .find(|g| &g.group_id == group_id)
            .map_or_else(String::new, |g| helpers::sanitize_for_display(&g.name));
        let now = rekindle_utils::timestamp_ms_i64();
        let mut rows: Vec<(i64, Line<'static>)> = messages
            .iter()
            .flat_map(|m| {
                if let Some(seconds) = m.timer_notice_seconds {
                    let who = if m.is_own {
                        "You".to_string()
                    } else {
                        self.label(group_id, &m.sender_id)
                    };
                    let line = Line::styled(
                        format!(
                            "-- {} --",
                            helpers::sanitize_for_display(&notice_text(&who, seconds))
                        ),
                        Style::new().add_modifier(Modifier::DIM | Modifier::ITALIC),
                    );
                    return vec![(m.timestamp, line)];
                }
                let who = if m.is_own {
                    "you".to_string()
                } else {
                    helpers::sanitize_for_display(&self.label(group_id, &m.sender_id))
                };
                let time = helpers::format_time_short(m.timestamp.cast_unsigned());
                let state = match (m.is_own, m.send_state) {
                    (true, SendState::Queued) => " (sending)",
                    (true, SendState::Failed) => " (failed)",
                    _ => "",
                };
                // The countdown to the Host's `expire_at` (plan E7.1).
                let expiry = m
                    .expire_at
                    .map(|at| format!(" (disappears in {})", remaining_short(at, now)))
                    .unwrap_or_default();
                let line = Line::raw(format!(
                    "[{time}] {who}: {}{state}{expiry}",
                    helpers::sanitize_for_display(&m.body)
                ));
                let mut row = vec![(m.timestamp, line)];
                // The Host still trying (plan SR2b): under our message, with
                // its timestamp, so the stable sort keeps it there.
                if let Some(retry) = m
                    .message_id
                    .as_ref()
                    .filter(|_| m.is_own)
                    .and_then(|id| self.retries.get(id))
                {
                    row.push((
                        m.timestamp,
                        Line::styled(
                            format!("    {}", helpers::delivery_retry_text(retry)),
                            Style::new().add_modifier(Modifier::ITALIC),
                        ),
                    ));
                }
                row
            })
            .collect();
        rows.sort_by_key(|(at, _)| *at);
        let lines = rows.into_iter().map(|(_, line)| line).collect();
        (format!(" {title} "), lines)
    }
}

impl GroupDmListView {
    /// The open group's disappearing timer changed (its notice joins the
    /// timeline) or the Host deleted expired messages; plan E7.1.
    fn on_disappearing(&mut self, event: &ChannelMessageEvent) {
        let Some(open) = self.open.as_ref().map(|(id, _)| id.clone()) else {
            return;
        };
        match event {
            ChannelMessageEvent::DisappearingTimerChanged {
                context: ConversationContext::GroupDm { group_id },
                seconds,
                set_by,
                client_id,
                timestamp,
            } if *group_id == open => {
                // Named by the member's label, as the stored row's reload is
                // (the roster carries no own-key mark; our label is the Host's).
                if let Some((_, messages)) = &mut self.open {
                    messages.push(notice_row(set_by, *seconds, client_id, *timestamp));
                }
            }
            ChannelMessageEvent::MessagesExpired {
                context: ConversationContext::GroupDm { group_id },
                client_ids,
            } if *group_id == open => {
                if let Some((_, messages)) = &mut self.open {
                    messages.retain(|m| {
                        m.message_id
                            .as_ref()
                            .is_none_or(|id| !client_ids.contains(id))
                    });
                }
            }
            _ => {}
        }
    }
}

/// The live notice of a timer change, shaped as the stored row a reload
/// returns (`messages.timer_notice_seconds`, plan E7.1).
fn notice_row(set_by: &str, seconds: u32, client_id: &str, timestamp: i64) -> DmMessage {
    DmMessage {
        id: 0,
        message_id: Some(client_id.to_owned()),
        sender_id: set_by.to_owned(),
        body: String::new(),
        timestamp,
        is_own: false,
        send_state: SendState::Delivered,
        history_gap_through: None,
        expire_seconds: seconds,
        expire_at: None,
        timer_notice_seconds: Some(seconds),
    }
}

impl View for GroupDmListView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, _theme: &ThemeManager) -> Result<()> {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)])
                .areas(area);
        let list = List::new(self.list_items())
            .block(
                Block::bordered()
                    .title(" Groups ")
                    .border_style(super::focus_border(&self.focus, FocusId::DmList)),
            )
            .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
        frame.render_stateful_widget(list, left, &mut self.list_state);
        let (title, lines) = self.message_lines();
        let messages = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .title(title)
                .border_style(super::focus_border(&self.focus, FocusId::MessageList)),
        );
        let [messages_area, input_area] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(3)]).areas(right);
        frame.render_widget(messages, messages_area);
        self.input_box
            .set_focused(self.focus.is_focused(FocusId::InputBox));
        self.input_box.draw(frame, input_area)?;
        Ok(())
    }

    fn update(&mut self, action: Action) -> Result<Option<Action>> {
        match action {
            Action::InputSubmit => {
                let text = self.input_box.content();
                let Some((group_id, _)) = &self.open else {
                    return Ok(None);
                };
                if !text.trim().is_empty() && !self.input_box.is_over_limit() {
                    let send = Action::SendGroupDm {
                        group_id: group_id.clone(),
                        text,
                    };
                    self.input_box.clear();
                    return Ok(Some(send));
                }
            }
            Action::EnterInputMode => self.focus.set(FocusId::InputBox),
            Action::ExitInputMode => self.focus.set(FocusId::DmList),
            _ => {}
        }
        Ok(None)
    }

    fn on_command_result(&mut self, result: CommandResult) -> Result<()> {
        match result {
            CommandResult::GroupDmsLoaded { groups } => self.set_groups(groups),
            CommandResult::GroupMessagesLoaded { group_id, messages } => {
                self.open = Some((group_id, messages));
            }
            _ => {}
        }
        Ok(())
    }

    /// The open group's messages follow its stream: a member's message is
    /// appended once, and our echo's row takes its send status.
    fn on_subscription_event(&mut self, event: &SubscriptionEvent) -> Result<()> {
        if let SubscriptionEvent::ChannelMessage(event) = event {
            self.on_disappearing(event);
            return Ok(());
        }
        let SubscriptionEvent::GroupDm(event) = event else {
            return Ok(());
        };
        let Some((open, messages)) = &mut self.open else {
            return Ok(());
        };
        match event {
            GroupDmEvent::Message {
                group_id,
                message_id,
                sender,
                body,
                timestamp,
                ..
            } if group_id == open => {
                let held = messages
                    .iter()
                    .any(|m| m.message_id.as_deref() == Some(message_id.as_str()));
                if !held {
                    messages.push(DmMessage {
                        id: 0,
                        message_id: Some(message_id.clone()),
                        sender_id: sender.public_key.clone(),
                        body: body.clone(),
                        timestamp: *timestamp,
                        is_own: false,
                        send_state: SendState::Delivered,
                        history_gap_through: None,
                        expire_seconds: 0,
                        expire_at: None,
                        timer_notice_seconds: None,
                    });
                }
            }
            GroupDmEvent::SendStatus {
                group_id,
                client_id,
                state,
                retry,
            } if group_id == open => {
                match retry {
                    Some(r) => {
                        self.retries.insert(client_id.clone(), *r);
                    }
                    None => {
                        self.retries.remove(client_id);
                    }
                }
                for m in messages
                    .iter_mut()
                    .filter(|m| m.message_id.as_deref() == Some(client_id.as_str()))
                {
                    m.send_state = *state;
                }
            }
            _ => {}
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
            code => {
                let g = self.selected()?;
                let group_id = g.group_id.clone();
                match (code, g.pending) {
                    (KeyCode::Enter, false) => Some(Action::ShowGroupMessages { group_id }),
                    (KeyCode::Char('a'), true) => Some(Action::AcceptGroupInvite { group_id }),
                    (KeyCode::Char('x'), true) => Some(Action::DeclineGroupInvite { group_id }),
                    (KeyCode::Char('L'), false) => Some(Action::LeaveGroup { group_id }),
                    _ => None,
                }
            }
        }
    }

    fn focus_ring(&mut self) -> &mut FocusRing {
        &mut self.focus
    }
}

#[cfg(test)]
#[path = "group_dm_tests.rs"]
mod tests;
