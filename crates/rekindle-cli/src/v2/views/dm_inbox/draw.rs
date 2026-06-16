//! DM inbox rendering — two-pane: thread list | selected thread messages + compose.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

use crate::v2::helpers;
use crate::v2::tui::components::message_list::cache;
use crate::v2::tui::components::message_list::render as msg_render;
use crate::v2::tui::components::unread_badge;
use crate::v2::tui::focus::FocusId;
use crate::v2::tui::state::navigation::ViewKindTag;
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;
use crate::v2::tui::theme::ThemeManager;

pub fn draw(
    state: &TuiState,
    frame: &mut Frame,
    area: Rect,
    theme: &ThemeManager,
    caches: &mut RenderCaches,
) {
    let [list_area, thread_area] = Layout::horizontal([
        Constraint::Length(28),
        Constraint::Fill(1),
    ]).areas(area);

    let targets = caches.click_targets.entry(ViewKindTag::DmInbox).or_default();
    targets.insert(FocusId::DmList, list_area);

    let list_focused = state.nav.focus_ring.is_focused(FocusId::DmList);
    let list_border = if list_focused { theme.focused_border() } else { theme.unfocused_border() };
    let list_block = Block::bordered()
        .title(format!(" DMs ({}) ", state.dm.threads.len()))
        .border_style(list_border);

    if state.dm.threads.is_empty() {
        frame.render_widget(
            Paragraph::new("  No conversations.").style(theme.style("dim")).block(list_block),
            list_area,
        );
    } else {
        let selected_index = state.session.dm_selected_peer.as_deref()
            .and_then(|pk| state.dm.threads.get_index_of(pk));
        caches.dm_inbox_list.select(selected_index);

        let items: Vec<ListItem<'static>> = state.dm.threads.values().map(|thread| {
            let name = helpers::sanitize_for_display(&thread.peer_name);
            let is_invite = thread.peer_name.starts_with("[invite]");
            let name_style = if is_invite {
                theme.style("dim").add_modifier(Modifier::ITALIC)
            } else {
                theme.style("accent")
            };
            let badge = unread_badge::unread_span(thread.unread_count, theme);
            let typing = if thread.is_typing { " \u{270e}" } else { "" };
            let time = thread.last_message_at
                .filter(|&t| t > 0)
                .map(|t| helpers::format_time_short(t, &state.timezone))
                .unwrap_or_default();

            let typing_span = theme.span("dim", typing);
            let time_label = format!("  {time}");
            let time_span = theme.span("dim", &time_label);
            ListItem::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(name, name_style),
                badge,
                typing_span,
                time_span,
            ]))
        }).collect();

        let item_count = items.len();
        frame.render_stateful_widget(
            List::new(items)
                .block(list_block)
                .highlight_style(theme.style("selected"))
                .highlight_symbol("\u{25b8} ")
                .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
                .scroll_padding(1),
            list_area,
            &mut caches.dm_inbox_list,
        );
        let mut scrollbar_state = ScrollbarState::new(item_count)
            .position(caches.dm_inbox_list.selected().unwrap_or(0));
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            list_area,
            &mut scrollbar_state,
        );
    }

    let [msg_area, input_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(3),
    ]).areas(thread_area);

    targets.insert(FocusId::MessageList, msg_area);
    targets.insert(FocusId::InputBox, input_area);

    let selected_peer = state.session.dm_selected_peer.as_deref()
        .and_then(|pk| state.dm.threads.get(pk));

    if let Some(thread) = selected_peer {
        let msg_focused = state.nav.focus_ring.is_focused(FocusId::MessageList);

        tracing::trace!(
            peer = &thread.peer_key[..12.min(thread.peer_key.len())],
            msg_count = thread.messages().len(),
            loaded = thread.loaded,
            loading = thread.loading,
            generation = thread.generation(),
            "tui draw: dm_inbox selected thread state"
        );
        if thread.messages().is_empty() {
            let border = if msg_focused { theme.focused_border() } else { theme.unfocused_border() };
            let block = Block::bordered()
                .title(format!(" {} ", helpers::sanitize_for_display(&thread.peer_name)))
                .border_style(border);

            let text = if thread.loading {
                "  Loading messages..."
            } else if !thread.loaded {
                "  Select to load messages"
            } else {
                "  No messages yet."
            };
            frame.render_widget(
                Paragraph::new(text).style(theme.style("dim")).block(block),
                msg_area,
            );
        } else {
            let messages = thread.messages();
            let generation = thread.generation();
            let msg_count = messages.len();
            let cache_entry = caches.dm_thread.entry(thread.peer_key.clone()).or_default();
            msg_render::render_messages(
                msg_count,
                cache_entry,
                generation,
                None,
                || cache::build_rendered_dm(messages),
                frame,
                msg_area,
                &helpers::sanitize_for_display(&thread.peer_name),
                msg_focused,
                theme,
                &state.timezone,
            );
        }

        let input_focused = state.nav.focus_ring.is_focused(FocusId::InputBox);
        if let Some(input) = state.session.dm_inputs.get(
            state.session.dm_selected_peer.as_deref().unwrap_or(""),
        ) {
            input.draw(frame, input_area, input_focused);
        } else {
            let border = if input_focused { theme.focused_border() } else { theme.unfocused_border() };
            let block = Block::bordered().title(" Compose ").border_style(border);
            frame.render_widget(
                Paragraph::new("  Press 'i' to compose").style(theme.style("dim")).block(block),
                input_area,
            );
        }
    } else {
        let border = theme.unfocused_border();
        let block = Block::bordered().title(" Messages ").border_style(border);
        frame.render_widget(
            Paragraph::new("  Select a conversation.").style(theme.style("dim")).block(block),
            msg_area,
        );

        let block = Block::bordered().title(" Compose ").border_style(border);
        frame.render_widget(
            Paragraph::new("  Press 'i' to compose").style(theme.style("dim")).block(block),
            input_area,
        );
    }
}
