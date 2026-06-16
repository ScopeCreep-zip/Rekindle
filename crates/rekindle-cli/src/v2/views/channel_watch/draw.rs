//! Channel watch rendering — responsive 3-pane with split DM, thread panel, pins.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::v2::helpers;
use crate::v2::tui::components::message_list::cache;
use crate::v2::tui::components::message_list::render as msg_render;
use crate::v2::tui::components::typing_indicator;
use crate::v2::tui::focus::FocusId;
use crate::v2::tui::state::channels::ChannelKey;
use crate::v2::tui::state::navigation::ViewKindTag;
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;
use crate::v2::tui::theme::ThemeManager;

const SIDEBAR_COLLAPSE_WIDTH: u16 = 60;
const PEER_LIST_COLLAPSE_WIDTH: u16 = 100;
const SIDEBAR_WIDTH: u16 = 22;
const PEER_LIST_WIDTH: u16 = 18;

pub fn draw(
    state: &TuiState,
    frame: &mut Frame,
    area: Rect,
    theme: &ThemeManager,
    community: &str,
    channel: &str,
    caches: &mut RenderCaches,
) {
    let key = ChannelKey { community: community.to_string(), channel: channel.to_string() };
    let ch = state.channels.channels.get(&key);

    let show_sidebar = state.nav.sidebar_visible && area.width >= SIDEBAR_COLLAPSE_WIDTH;
    let show_peers = area.width >= PEER_LIST_COLLAPSE_WIDTH;

    let mut h_constraints: Vec<Constraint> = Vec::new();
    if show_sidebar { h_constraints.push(Constraint::Length(SIDEBAR_WIDTH)); }
    h_constraints.push(Constraint::Fill(1));
    if show_peers { h_constraints.push(Constraint::Length(PEER_LIST_WIDTH)); }

    let h_areas = Layout::horizontal(h_constraints).split(area);
    let mut col = 0;

    let mut sidebar_rect: Option<Rect> = None;
    if show_sidebar {
        let sidebar_area = h_areas[col];
        sidebar_rect = Some(sidebar_area);
        let sidebar_focused = state.nav.focus_ring.is_focused(FocusId::ChannelTree);
        let border = if sidebar_focused { theme.focused_border() } else { theme.unfocused_border() };
        let block = Block::bordered().title(" Channels ").border_style(border);

        if let Some(detail) = state.communities.details.get(community) {
            let channel_names: Vec<String> = detail.channels.iter()
                .map(|c| format!("  #{}", c.name))
                .collect();
            let text = if channel_names.is_empty() {
                "  No channels".to_string()
            } else {
                channel_names.join("\n")
            };
            frame.render_widget(Paragraph::new(text).block(block), sidebar_area);
        } else {
            frame.render_widget(Paragraph::new("  Loading...").style(theme.style("dim")).block(block), sidebar_area);
        }
        col += 1;
    }

    let center_area = h_areas[col];
    col += 1;

    let has_split_dm = ch.and_then(|c| c.active_split_dm.as_ref()).is_some();

    if has_split_dm {
        let [channel_half, dm_half] = Layout::horizontal([
            Constraint::Percentage(50), Constraint::Percentage(50),
        ]).areas(center_area);
        render_channel_pane(state, frame, channel_half, theme, community, channel, ch, caches);
        if let Some(split_peer) = ch.and_then(|c| c.active_split_dm.as_deref()) {
            render_split_dm_pane(state, frame, dm_half, theme, split_peer, caches);
        }
    } else {
        render_channel_pane(state, frame, center_area, theme, community, channel, ch, caches);
    }

    let mut peer_rect: Option<Rect> = None;
    if show_peers {
        let peer_area = h_areas[col];
        peer_rect = Some(peer_area);
        let peer_focused = state.nav.focus_ring.is_focused(FocusId::PeerList);
        let border = if peer_focused { theme.focused_border() } else { theme.unfocused_border() };

        if let Some(members) = state.communities.members.get(community) {
            let names: Vec<String> = members.iter()
                .map(|m| format!("  {}", m.member.display_name))
                .collect();
            let block = Block::bordered()
                .title(format!(" Members ({}) ", members.len()))
                .border_style(border);
            frame.render_widget(Paragraph::new(names.join("\n")).block(block), peer_area);
        } else {
            let block = Block::bordered().title(" Members ").border_style(border);
            frame.render_widget(Paragraph::new("  Loading...").style(theme.style("dim")).block(block), peer_area);
        }
    }

    let targets = caches.click_targets.entry(ViewKindTag::ChannelWatch).or_default();
    if let Some(r) = sidebar_rect { targets.insert(FocusId::ChannelTree, r); }
    if let Some(r) = peer_rect { targets.insert(FocusId::PeerList, r); }
}

fn render_channel_pane(
    state: &TuiState,
    frame: &mut Frame,
    area: Rect,
    theme: &ThemeManager,
    community: &str,
    channel: &str,
    ch: Option<&crate::v2::tui::state::channels::ChannelViewState>,
    caches: &mut RenderCaches,
) {
    let typing_names: Vec<String> = ch.map_or(Vec::new(), |c| {
        c.typing_indicators.keys().map(|k| helpers::abbreviate_key(k)).collect()
    });
    let typing_height = u16::from(!typing_names.is_empty());
    let pins_height = ch.map_or(0u16, |c| {
        if c.has_pins() { 1 } else { 0 }
    });

    let [msg_area, pins_area, typing_area, input_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(pins_height),
        Constraint::Length(typing_height),
        Constraint::Length(3),
    ]).areas(area);

    let targets = caches.click_targets.entry(ViewKindTag::ChannelWatch).or_default();
    targets.insert(FocusId::MessageList, msg_area);
    targets.insert(FocusId::InputBox, input_area);

    let msg_focused = state.nav.focus_ring.is_focused(FocusId::MessageList);
    let key = ChannelKey { community: community.to_string(), channel: channel.to_string() };

    if let Some(ch) = ch {
        tracing::trace!(
            channel, msg_count = ch.messages().len(),
            loaded = ch.loaded, loading = ch.loading,
            generation = ch.generation(),
            "tui draw: channel_watch state"
        );
        if ch.messages().is_empty() {
            let border = if msg_focused { theme.focused_border() } else { theme.unfocused_border() };
            let block = Block::bordered().title(format!(" #{channel} ")).border_style(border);
            let text = if ch.loading { "  Loading..." } else { "  No messages yet." };
            frame.render_widget(Paragraph::new(text).style(theme.style("dim")).block(block), msg_area);
        } else {
            let messages = ch.messages();
            let reactions = ch.reactions();
            let gen = ch.generation();
            let msg_count = messages.len();
            let cache_entry = caches.channel.entry(key.clone()).or_default();
            msg_render::render_messages(
                msg_count,
                cache_entry,
                gen,
                None,
                || cache::build_rendered_channel(messages, reactions),
                frame,
                msg_area,
                &format!("#{channel}"),
                msg_focused,
                theme,
                &state.timezone,
            );
        }
    } else {
        let border = if msg_focused { theme.focused_border() } else { theme.unfocused_border() };
        let block = Block::bordered().title(format!(" #{channel} ")).border_style(border);
        frame.render_widget(Paragraph::new("  Loading...").style(theme.style("dim")).block(block), msg_area);
    }

    if let Some(ch_data) = ch {
        if let Some(pins) = ch_data.pins() {
            if !pins.is_empty() {
                let first = &pins[0];
                let age = if first.pinned_at > 0 && state.wall_clock_ms > first.pinned_at {
                    let elapsed = std::time::Duration::from_millis(state.wall_clock_ms - first.pinned_at);
                    helpers::format_duration_ago(elapsed)
                } else {
                    String::new()
                };
                let channel_note = if first.channel_id != channel { format!(" in #{}", first.channel_id) } else { String::new() };
                let pin_count = format!("  \u{1f4cc} {} pinned{channel_note}", pins.len());
                let pin_detail = format!("  \u{2014} {} by {}  {age}",
                    first.body_preview, helpers::abbreviate_key(&first.pinned_by));
                let count_span = theme.span("dim", &pin_count);
                let detail_span = theme.span("dim", &pin_detail);
                let hint_span = theme.span("dim", "  [p] toggle  [u] unpin");
                frame.render_widget(
                    Paragraph::new(Line::from(vec![count_span, detail_span, hint_span])),
                    pins_area,
                );
            }
        }
    }

    if !typing_names.is_empty() {
        let text = typing_indicator::format_typing_compact(&typing_names);
        frame.render_widget(
            Paragraph::new(format!("  {text}")).style(theme.style("dim").add_modifier(Modifier::ITALIC)),
            typing_area,
        );
    }

    let input_focused = state.nav.focus_ring.is_focused(FocusId::InputBox);
    if let Some(input) = state.session.channel_inputs.get(&key) {
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

fn render_split_dm_pane(
    state: &TuiState,
    frame: &mut Frame,
    area: Rect,
    theme: &ThemeManager,
    peer_key: &str,
    caches: &mut RenderCaches,
) {
    let thread = state.dm.threads.get(peer_key);
    let peer_name = thread.map_or("DM", |t| t.peer_name.as_str());
    let is_typing = thread.map_or(false, |t| t.is_typing);
    let typing_height = u16::from(is_typing);

    let [msg_area, typing_area, input_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(typing_height),
        Constraint::Length(3),
    ]).areas(area);

    let targets = caches.click_targets.entry(ViewKindTag::ChannelWatch).or_default();
    targets.insert(FocusId::SplitDmMessages, msg_area);
    targets.insert(FocusId::SplitDmInput, input_area);

    let msg_focused = state.nav.focus_ring.is_focused(FocusId::SplitDmMessages);

    if let Some(thread) = thread {
        if !thread.messages().is_empty() {
            let messages = thread.messages();
            let gen = thread.generation();
            let msg_count = messages.len();
            let cache_entry = caches.split_dm.entry(peer_key.to_string()).or_default();
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
        } else {
            let border = if msg_focused { theme.focused_border() } else { theme.unfocused_border() };
            let block = Block::bordered()
                .title(format!(" {} ", helpers::sanitize_for_display(peer_name)))
                .border_style(border);
            frame.render_widget(Paragraph::new("  No messages yet.").style(theme.style("dim")).block(block), msg_area);
        }
    } else {
        let border = if msg_focused { theme.focused_border() } else { theme.unfocused_border() };
        let block = Block::bordered().title(format!(" {} ", peer_name)).border_style(border);
        frame.render_widget(Paragraph::new("  Loading...").style(theme.style("dim")).block(block), msg_area);
    }

    if is_typing {
        let text = typing_indicator::format_typing_compact(std::slice::from_ref(&peer_name.to_string()));
        frame.render_widget(
            Paragraph::new(format!("  {text}")).style(theme.style("dim").add_modifier(Modifier::ITALIC)),
            typing_area,
        );
    }

    let input_focused = state.nav.focus_ring.is_focused(FocusId::SplitDmInput);
    if let Some(input) = state.session.split_dm_inputs.get(peer_key) {
        input.draw(frame, input_area, input_focused);
    } else {
        let border = if input_focused { theme.focused_border() } else { theme.unfocused_border() };
        let block = Block::bordered().title(" DM ").border_style(border);
        frame.render_widget(
            Paragraph::new("  Press 'i' to compose").style(theme.style("dim")).block(block),
            input_area,
        );
    }
}
