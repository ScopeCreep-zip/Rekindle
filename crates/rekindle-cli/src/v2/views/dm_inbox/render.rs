//! DM inbox rendering — two-pane: conversation list | thread + input.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};
use ratatui::Frame;

use super::DmInboxView;
use crate::v2::helpers;
use crate::v2::tui::components::{Component, unread_badge};
use crate::v2::tui::focus::FocusId;
use crate::v2::tui::theme::ThemeManager;

pub fn draw(view: &mut DmInboxView, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
    let [list_area, thread_area] = Layout::horizontal([
        Constraint::Length(28), Constraint::Fill(1),
    ]).areas(area);

    view.click_rects.clear();
    view.click_rects.insert(FocusId::DmList, list_area);

    let list_block = Block::bordered()
        .title(format!(" DMs ({}) ", view.threads.len()))
        .border_style(if view.focus.is_focused(FocusId::DmList) { Style::new() } else { Style::new().dim() });

    if view.threads.is_empty() {
        frame.render_widget(Paragraph::new("  No conversations.").style(Style::new().dim()).block(list_block), list_area);
    } else {
        let items = build_thread_items(view, theme);
        frame.render_stateful_widget(
            List::new(items).block(list_block).highlight_style(Style::new().reversed()),
            list_area, &mut view.thread_list_state,
        );
    }

    let [msg_area, input_area] = Layout::vertical([Constraint::Fill(1), Constraint::Length(3)]).areas(thread_area);

    if let Some(idx) = view.thread_list_state.selected() {
        if let Some(thread) = view.threads.get(idx) {
            render_thread(frame, msg_area, thread, view.focus.is_focused(FocusId::MessageList));
        }
    } else {
        let block = Block::bordered().title(" Messages ").border_style(Style::new().dim());
        frame.render_widget(Paragraph::new("  Select a conversation.").style(Style::new().dim()).block(block), msg_area);
    }

    view.click_rects.insert(FocusId::MessageList, msg_area);
    view.input_box.set_focused(view.focus.is_focused(FocusId::InputBox));
    view.input_box.draw(frame, input_area);
    view.click_rects.insert(FocusId::InputBox, input_area);
}

fn build_thread_items(view: &DmInboxView, theme: &ThemeManager) -> Vec<ListItem<'static>> {
    view.threads.iter().map(|thread| {
        let name = helpers::sanitize_for_display(&thread.peer_name);
        #[allow(clippy::cast_possible_truncation)]
        let unread = thread.messages.iter().filter(|m| !m.is_self).count() as u32;
        let time = if thread.last_message_at > 0 { helpers::format_time_short(thread.last_message_at) } else { String::new() };
        let badge = unread_badge::unread_span(unread, theme);
        let glyph = if unread > 0 { if view.use_unicode { "● " } else { "* " } } else { "  " };

        // Last message delivery status (for self-sent messages)
        let delivery = thread.messages.last()
            .filter(|m| m.is_self)
            .map_or("", |_| " ●");

        ListItem::new(Line::from(vec![
            Span::raw(format!("  {glyph}")),
            Span::styled(name, Style::new().bold()),
            badge,
            Span::styled(delivery, Style::new().dim()),
            Span::styled(format!("  {time}"), Style::new().dim()),
        ]))
    }).collect()
}

fn render_thread(frame: &mut Frame, area: Rect, thread: &rekindle_types::display::DmThreadDisplay, focused: bool) {
    let block = Block::bordered()
        .title(format!(" {} ", helpers::sanitize_for_display(&thread.peer_name)))
        .border_style(if focused { Style::new() } else { Style::new().dim() });

    if thread.messages.is_empty() {
        frame.render_widget(Paragraph::new("  No messages yet.").style(Style::new().dim()).block(block), area);
        return;
    }

    let lines: Vec<Line<'_>> = thread.messages.iter().map(|msg| {
        let sender = if msg.is_self { "you" } else { &msg.sender_name };
        Line::from(vec![
            Span::styled(format!("  [{}] ", helpers::format_time_short(msg.timestamp)), Style::new().dim()),
            Span::styled(format!("{}: ", helpers::sanitize_for_display(sender)), Style::new().bold()),
            Span::raw(helpers::sanitize_for_display(&msg.body)),
        ])
    }).collect();

    frame.render_widget(Paragraph::new(lines).block(block), area);
}
