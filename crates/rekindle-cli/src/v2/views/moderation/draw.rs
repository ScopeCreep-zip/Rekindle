//! Moderation panel rendering — three tabs: Members, Bans, Pending Queue.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

use crate::v2::helpers;
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
    community: &str,
    caches: &mut RenderCaches,
) {
    let targets = caches.click_targets.entry(ViewKindTag::Moderation).or_default();
    targets.insert(FocusId::MessageList, area);

    let name = state.communities.name_for(community);

    if !state.communities.details.contains_key(community) {
        let block = Block::bordered()
            .title(format!(" Moderation: {name} "))
            .border_style(theme.focused_border());
        frame.render_widget(
            Paragraph::new("  Loading moderation data...").style(theme.style("dim")).block(block),
            area,
        );
        return;
    }

    let [tab_area, content_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
    ]).areas(area);

    let tab = state.ephemeral.moderation_tab;
    let tab_names = ["Members", "Bans", "Queue"];
    let tab_spans: Vec<Span<'_>> = tab_names.iter().enumerate().map(|(i, name)| {
        if i as u8 == tab {
            Span::styled(format!(" [{name}] "), theme.style("accent").add_modifier(Modifier::BOLD))
        } else {
            {
                let label = format!("  {name}  ");
                theme.span("dim", &label)
            }
        }
    }).collect();
    frame.render_widget(Paragraph::new(Line::from(tab_spans)), tab_area);

    let block = Block::bordered()
        .title(format!(" Moderation: {name} "))
        .border_style(theme.focused_border());

    let selected = state.ephemeral.moderation_selected;

    match tab {
        0 => {
            if let Some(members) = state.communities.members.get(community) {
                let items: Vec<ListItem<'static>> = members.iter().map(|m| {
                    let (glyph, _, _) = theme.presence_indicator(&m.status);
                    let role = m.role_name.as_deref().unwrap_or("");
                    let timeout = m.member.timeout_until
                        .map_or(String::new(), |t| format!("  [timeout until {t}]"));
                    ListItem::new(Line::from(vec![
                        Span::raw(format!("  {glyph} ")),
                        theme.span("accent", &m.member.display_name),
                        Span::styled(
                            if role.is_empty() { String::new() } else { format!("  [{role}]") },
                            theme.style("dim"),
                        ),
                        theme.span("dim", &timeout),
                        theme.span("dim", "    [K]ick [b]an [t]imeout"),
                    ]))
                }).collect();
                let item_count = items.len();
                let max = item_count.saturating_sub(1);
                caches.moderation_list.select(Some(selected.min(max)));
                frame.render_stateful_widget(
                    List::new(items)
                        .block(block)
                        .highlight_style(theme.style("selected"))
                        .highlight_symbol("\u{25b8} ")
                        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
                        .scroll_padding(1),
                    content_area,
                    &mut caches.moderation_list,
                );
                let mut scrollbar_state = ScrollbarState::new(item_count)
                    .position(caches.moderation_list.selected().unwrap_or(0));
                frame.render_stateful_widget(
                    Scrollbar::new(ScrollbarOrientation::VerticalRight)
                        .begin_symbol(None)
                        .end_symbol(None),
                    content_area,
                    &mut scrollbar_state,
                );
            } else {
                frame.render_widget(
                    Paragraph::new("  No members loaded.").style(theme.style("dim")).block(block),
                    content_area,
                );
            }
        }
        1 => {
            let bans = state.communities.bans.get(community);
            if let Some(bans) = bans {
                let items: Vec<ListItem<'static>> = bans.iter().map(|b| {
                    let pseudo = b.get("pseudonymKey").and_then(|v| v.as_str()).unwrap_or("?");
                    let reason = b.get("reason").and_then(|v| v.as_str()).unwrap_or("no reason");
                    let by = b.get("bannedBy").and_then(|v| v.as_str()).unwrap_or("?");
                    ListItem::new(Line::from(vec![
                        Span::raw(format!("  {} ", helpers::abbreviate_key(pseudo))),
                        Span::styled(
                            format!("\u{2014} banned by {} \u{2014} \"{reason}\"", helpers::abbreviate_key(by)),
                            theme.style("dim"),
                        ),
                        theme.span("dim", "    [u]nban"),
                    ]))
                }).collect();
                let item_count = items.len();
                let max = item_count.saturating_sub(1);
                caches.moderation_list.select(Some(selected.min(max)));
                frame.render_stateful_widget(
                    List::new(items)
                        .block(block)
                        .highlight_style(theme.style("selected"))
                        .highlight_symbol("\u{25b8} ")
                        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
                        .scroll_padding(1),
                    content_area,
                    &mut caches.moderation_list,
                );
                let mut scrollbar_state = ScrollbarState::new(item_count)
                    .position(caches.moderation_list.selected().unwrap_or(0));
                frame.render_stateful_widget(
                    Scrollbar::new(ScrollbarOrientation::VerticalRight)
                        .begin_symbol(None)
                        .end_symbol(None),
                    content_area,
                    &mut scrollbar_state,
                );
            } else {
                frame.render_widget(
                    Paragraph::new("  No bans.").style(theme.style("dim")).block(block),
                    content_area,
                );
            }
        }
        2 => {
            let pending_members = state.communities.pending_members.get(community);
            if let Some(queue) = pending_members {
                let items: Vec<ListItem<'static>> = queue.iter().map(|p| {
                    let name = p.get("displayName").or_else(|| p.get("display_name"))
                        .and_then(|v| v.as_str()).unwrap_or("?");
                    let pseudo = p.get("requesterPseudonymHex").or_else(|| p.get("requester_pseudonym_hex"))
                        .and_then(|v| v.as_str()).unwrap_or("?");
                    let has_invite = p.get("inviteCodeHash").or_else(|| p.get("invite_code_hash")).is_some();
                    ListItem::new(Line::from(vec![
                        Span::raw(format!("  \u{2192} {name} ")),
                        Span::styled(format!("({}) ", helpers::abbreviate_key(pseudo)), theme.style("dim")),
                        Span::styled(
                            if has_invite { "invite: yes" } else { "invite: no" },
                            theme.style("dim"),
                        ),
                        theme.span("dim", "    [a]pprove [r]eject"),
                    ]))
                }).collect();
                let item_count = items.len();
                let max = item_count.saturating_sub(1);
                caches.moderation_list.select(Some(selected.min(max)));
                frame.render_stateful_widget(
                    List::new(items)
                        .block(block)
                        .highlight_style(theme.style("selected"))
                        .highlight_symbol("\u{25b8} ")
                        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
                        .scroll_padding(1),
                    content_area,
                    &mut caches.moderation_list,
                );
                let mut scrollbar_state = ScrollbarState::new(item_count)
                    .position(caches.moderation_list.selected().unwrap_or(0));
                frame.render_stateful_widget(
                    Scrollbar::new(ScrollbarOrientation::VerticalRight)
                        .begin_symbol(None)
                        .end_symbol(None),
                    content_area,
                    &mut scrollbar_state,
                );
            } else {
                frame.render_widget(
                    Paragraph::new("  No pending requests.").style(theme.style("dim")).block(block),
                    content_area,
                );
            }
        }
        _ => {}
    }
}

