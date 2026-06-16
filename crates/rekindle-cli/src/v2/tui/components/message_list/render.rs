//! Message list rendering — unified for both channel and DM messages.
//!
//! The single `render_messages` function handles grouping, delivery badges,
//! unread separator, encrypted placeholders, reply threading, reactions,
//! pins, threads, patch fences, date separators, file path highlighting,
//! scrollbar, highlight symbol, and scroll-to-latest.
//!
//! The caller provides a `build_rendered` closure that runs ONLY when the
//! cache needs rebuilding (generation mismatch). This closure performs the
//! type-specific conversion (channel messages + reactions vs DM messages)
//! into the unified `Vec<RenderedMessage>`.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

use rekindle_types::display::DeliveryStatus;

use crate::v2::helpers::{self, TimezoneMode};
use crate::v2::tui::state::render_caches::MessageRenderCache;
use crate::v2::tui::theme::ThemeManager;

use super::cache::{MessageGroup, RenderedMessage};

/// Render a message list from pre-built RenderedMessages.
///
/// This is the SINGLE render function for both channel and DM messages.
/// The `build_rendered` closure is called only when the cache needs rebuilding.
pub fn render_messages<F>(
    message_count: usize,
    cache: &mut MessageRenderCache,
    generation: u64,
    last_read_index: Option<usize>,
    build_rendered: F,
    frame: &mut Frame,
    area: Rect,
    title: &str,
    focused: bool,
    theme: &ThemeManager,
    tz: &TimezoneMode,
)
where
    F: FnOnce() -> Vec<RenderedMessage>,
{
    if message_count == 0 {
        tracing::trace!(title, "tui render: message_count=0, showing empty placeholder");
        let block = Block::bordered()
            .title(format!(" {title} "))
            .border_style(if focused { theme.focused_border() } else { theme.unfocused_border() });
        frame.render_widget(
            Paragraph::new("No messages yet.").style(theme.style("dim")).block(block),
            area,
        );
        return;
    }

    if cache.needs_rebuild(generation) {
        tracing::debug!(title, generation, cached_items = cache.items().len(), "tui render: cache rebuild triggered");
        let was_at_bottom = cache.is_at_bottom();
        let rendered = build_rendered();
        let items = build_list_items(&rendered, last_read_index, theme, tz);
        cache.rebuild(items, generation);
        if was_at_bottom {
            cache.list_state.select_last();
        }
    }

    let item_count = cache.items().len();
    let list = List::new(cache.items().to_vec())
        .block(
            Block::bordered()
                .title(format!(" {title} ({message_count} messages) "))
                .border_style(if focused { theme.focused_border() } else { theme.unfocused_border() }),
        )
        .highlight_style(theme.style("selected"))
        .highlight_symbol("\u{25b8} ")
        .highlight_spacing(ratatui::widgets::HighlightSpacing::Always)
        .scroll_padding(2);

    frame.render_stateful_widget(list, area, &mut cache.list_state);

    let mut scrollbar_state = ScrollbarState::new(item_count)
        .position(cache.list_state.selected().unwrap_or(0));
    frame.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        area,
        &mut scrollbar_state,
    );

    if !cache.is_at_bottom() && message_count > 0 {
        let hint = Paragraph::new(" \u{2191} scrolled \u{2014} press G to jump to latest ")
            .style(theme.style("dim").add_modifier(Modifier::ITALIC))
            .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(hint, Rect {
            x: area.x,
            y: area.bottom().saturating_sub(1),
            width: area.width,
            height: 1,
        });
    }
}

fn build_list_items(
    rendered: &[RenderedMessage],
    last_read_index: Option<usize>,
    theme: &ThemeManager,
    tz: &TimezoneMode,
) -> Vec<ListItem<'static>> {
    let mut items = Vec::with_capacity(rendered.len());
    let mut prev_day: Option<String> = None;

    let dim = theme.style("dim");
    let accent = theme.style("accent");
    let accent_bold = accent.add_modifier(Modifier::BOLD);
    let error_style = theme.style("error");
    let keyword_style = theme.style("keyword");
    let dim_italic = dim.add_modifier(Modifier::ITALIC);
    let dim_bold = dim.add_modifier(Modifier::BOLD);
    let success_fg = Style::new().fg(theme.color("success"));
    let error_fg = Style::new().fg(theme.color("error"));
    let info_fg = Style::new().fg(theme.color("info"));

    for (i, rm) in rendered.iter().enumerate() {
        if last_read_index == Some(i.saturating_sub(1)) && i > 0 {
            items.push(ListItem::new(Line::from(vec![
                Span::styled("\u{2500}\u{2500}\u{2500}\u{2500} ", dim),
                Span::styled("New", accent_bold),
                Span::styled(" \u{2500}\u{2500}\u{2500}\u{2500}", dim),
            ])));
        }

        let day = helpers::format_day(rm.msg.timestamp, tz);
        if prev_day.as_ref() != Some(&day) && !day.is_empty() {
            if prev_day.is_some() {
                let tz_suffix = if matches!(tz, TimezoneMode::Utc) { " (UTC)" } else { "" };
                items.push(ListItem::new(Line::from(Span::styled(
                    format!("\u{2500}\u{2500} {day}{tz_suffix} \u{2500}\u{2500}"),
                    dim,
                ))));
            }
            prev_day = Some(day);
        }

        let msg = &rm.msg;
        let mut lines = Vec::new();

        if rm.group == MessageGroup::Full {
            let author = helpers::sanitize_for_display(&msg.author_display_name);
            let time = helpers::format_time_short(msg.timestamp, tz);
            let (delivery_glyph, delivery_style) = match msg.delivery_status {
                DeliveryStatus::Sending => (" \u{25cb}", dim),
                DeliveryStatus::Confirmed => (" \u{25cf}", dim),
                DeliveryStatus::Failed => (" \u{2717}", error_style),
            };
            lines.push(Line::from(vec![
                Span::styled(author, accent),
                Span::styled(delivery_glyph, delivery_style),
                Span::raw("  "),
                Span::styled(format!("[{time}]"), dim),
            ]));
        }

        if msg.is_encrypted {
            let hint = msg.needs_mek
                .map(|gen| format!(" \u{2014} MEK gen {gen} needed"))
                .unwrap_or_default();
            lines.push(Line::from(vec![
                Span::styled(
                    format!("[encrypted, MEK gen {}]", msg.mek_generation),
                    dim_italic,
                ),
                Span::styled(hint, dim),
            ]));
        } else if rm.has_patch_fence {
            if let Some(diff_text) = crate::v2::patch::render::extract_patch_fence(&msg.body) {
                let patch = crate::v2::patch::render::parse_diff_to_patch(&diff_text);
                let patch_lines = crate::v2::patch::render::render_patch_lines(
                    &patch,
                    false,
                    success_fg,
                    error_fg,
                    dim,
                    dim_bold,
                    info_fg,
                    dim_bold,
                );
                let fence_start = msg.body.find("```patch").unwrap_or(0);
                if fence_start > 0 {
                    let preamble = helpers::sanitize_for_display(&msg.body[..fence_start]);
                    for line in preamble.trim().lines() {
                        lines.push(Line::from(format!("  {line}")));
                    }
                }
                lines.extend(patch_lines);
                if let Some(close_pos) = msg.body[fence_start..].find("```patch")
                    .and_then(|start| {
                        let content_start = msg.body[fence_start + start..].find('\n')? + fence_start + start + 1;
                        msg.body[content_start..].find("```").map(|end| content_start + end + 3)
                    })
                {
                    let postamble = helpers::sanitize_for_display(&msg.body[close_pos..]);
                    for line in postamble.trim().lines() {
                        if !line.is_empty() {
                            lines.push(Line::from(format!("  {line}")));
                        }
                    }
                }
            } else {
                let body = helpers::sanitize_for_display(&msg.body);
                for line in body.lines() {
                    lines.push(highlight_file_paths(line, keyword_style));
                }
            }
        } else {
            let body = helpers::sanitize_for_display(&msg.body);
            for line in body.lines() {
                lines.push(highlight_file_paths(line, keyword_style));
            }
        }

        if let Some(reply_seq) = msg.reply_to_sequence {
            lines.push(Line::from(vec![
                Span::styled("  \u{21b3} reply to ", dim),
                Span::styled(format!("#{reply_seq}"), dim_italic),
            ]));
        }

        if rm.pinned {
            lines.push(Line::from(Span::styled("  \u{1f4cc} pinned", dim)));
        }

        if let Some(ref reactions) = rm.reactions {
            let mut spans: Vec<Span<'_>> = vec![Span::raw("  ")];
            for (emoji, count) in reactions {
                spans.push(Span::styled(format!("{emoji} {count}  "), dim));
            }
            lines.push(Line::from(spans));
        }

        if rm.thread_reply_count > 0 {
            let label = if rm.thread_reply_count == 1 {
                "1 reply".to_string()
            } else {
                format!("{} replies", rm.thread_reply_count)
            };
            lines.push(Line::from(Span::styled(
                format!("  \u{1f9f5} {label}"),
                dim,
            )));
        }

        items.push(ListItem::new(lines));
    }
    items
}

const FILE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "py", "go", "c", "cpp", "h", "hpp",
    "java", "kt", "rb", "ex", "exs", "erl", "hs", "ml", "mli", "scala",
    "swift", "m", "mm", "zig", "nim", "lua", "sh", "bash", "zsh", "fish",
    "toml", "yaml", "yml", "json", "xml", "html", "css", "scss", "md",
    "txt", "cfg", "conf", "ini", "env", "lock", "nix", "flake",
    "dockerfile", "makefile", "cmake",
];

fn highlight_file_paths(line: &str, path_style: Style) -> Line<'static> {
    let indented = format!("  {line}");
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut last_end = 0;

    for (i, word) in indented.split_whitespace().enumerate() {
        let word_start = if i == 0 {
            indented.find(word).unwrap_or(0)
        } else {
            indented[last_end..].find(word).map_or(last_end, |p| last_end + p)
        };

        if word_start > last_end {
            spans.push(Span::raw(indented[last_end..word_start].to_string()));
        }

        let is_path = word.contains('/')
            || word.rsplit_once('.').is_some_and(|(_, ext)| {
                FILE_EXTENSIONS.contains(&ext.to_lowercase().as_str())
            });

        if is_path && word.len() > 2 {
            spans.push(Span::styled(
                word.to_string(),
                path_style.add_modifier(Modifier::UNDERLINED),
            ));
        } else {
            spans.push(Span::raw(word.to_string()));
        }

        last_end = word_start + word.len();
    }

    if last_end < indented.len() {
        spans.push(Span::raw(indented[last_end..].to_string()));
    }

    if spans.is_empty() {
        Line::from(indented)
    } else {
        Line::from(spans)
    }
}
