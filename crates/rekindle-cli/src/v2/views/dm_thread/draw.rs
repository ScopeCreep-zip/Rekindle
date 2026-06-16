//! DM thread rendering — standalone message thread with a single peer.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::v2::helpers;
use crate::v2::tui::components::message_list::cache;
use crate::v2::tui::components::message_list::render as msg_render;
use crate::v2::tui::components::typing_indicator;
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
    peer_key: &str,
    caches: &mut RenderCaches,
) {
    let thread = state.dm.threads.get(peer_key);
    let peer_name = thread.map_or("Unknown", |t| t.peer_name.as_str());
    let is_typing = thread.map_or(false, |t| t.is_typing);
    let typing_height = u16::from(is_typing);

    let [msg_area, typing_area, input_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(typing_height),
        Constraint::Length(3),
    ]).areas(area);

    let targets = caches.click_targets.entry(ViewKindTag::DmThread).or_default();
    targets.insert(FocusId::MessageList, msg_area);
    targets.insert(FocusId::InputBox, input_area);

    let msg_focused = state.nav.focus_ring.is_focused(FocusId::MessageList);

    if let Some(thread) = thread {
        tracing::trace!(
            peer_key, msg_count = thread.messages().len(),
            loaded = thread.loaded, loading = thread.loading,
            generation = thread.generation(),
            "tui draw: dm_thread state"
        );
        if thread.messages().is_empty() {
            let border = if msg_focused { theme.focused_border() } else { theme.unfocused_border() };
            let block = Block::bordered()
                .title(format!(" {} ", helpers::sanitize_for_display(peer_name)))
                .border_style(border);
            let text = if thread.loading {
                "  Loading messages..."
            } else if !thread.loaded {
                "  Loading..."
            } else {
                "  No messages yet."
            };
            frame.render_widget(Paragraph::new(text).style(theme.style("dim")).block(block), msg_area);
        } else {
            let messages = thread.messages();
            let gen = thread.generation();
            let msg_count = messages.len();
            let cache_entry = caches.dm_thread.entry(peer_key.to_string()).or_default();
            msg_render::render_messages(
                msg_count,
                cache_entry,
                gen,
                None,
                || cache::build_rendered_dm(messages),
                frame,
                msg_area,
                &helpers::sanitize_for_display(peer_name),
                msg_focused,
                theme,
                &state.timezone,
            );
        }
    } else {
        let border = if msg_focused { theme.focused_border() } else { theme.unfocused_border() };
        let block = Block::bordered()
            .title(format!(" {} ", helpers::sanitize_for_display(peer_name)))
            .border_style(border);
        frame.render_widget(Paragraph::new("  Loading...").style(theme.style("dim")).block(block), msg_area);
    }

    if is_typing {
        let text = typing_indicator::format_typing_compact(std::slice::from_ref(&peer_name.to_string()));
        frame.render_widget(
            Paragraph::new(format!("  {text}")).style(theme.style("dim").add_modifier(Modifier::ITALIC)),
            typing_area,
        );
    }

    let input_focused = state.nav.focus_ring.is_focused(FocusId::InputBox);
    if let Some(input) = state.session.dm_inputs.get(peer_key) {
        input.draw(frame, input_area, input_focused);
    } else {
        let border = if input_focused { theme.focused_border() } else { theme.unfocused_border() };
        let block = Block::bordered().title(" Compose ").border_style(border);
        frame.render_widget(
            Paragraph::new("  Press 'i' to compose").style(theme.style("dim")).block(block),
            input_area,
        );
    }
}
