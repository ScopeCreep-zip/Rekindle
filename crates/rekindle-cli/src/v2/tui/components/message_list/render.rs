//! Message list rendering — grouping, delivery indicators, unread separator,
//! encrypted placeholders, reply threading, generation-cached item building.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};
use ratatui::Frame;

use super::state::MessageList;
use super::types::MessageGroup;
use crate::v2::helpers;

impl MessageList {
    /// Build list items with grouping, delivery status, and formatting.
    pub fn build_items(&self) -> Vec<ListItem<'static>> {
        let mut items = Vec::with_capacity(self.len());

        for (i, rendered) in self.messages.iter().enumerate() {
            // Unread separator
            if self.last_read_index == Some(i.saturating_sub(1)) && i > 0 {
                items.push(ListItem::new(Line::from(vec![
                    Span::styled("──── ", Style::new().dim()),
                    Span::styled("New", Style::new().bold()),
                    Span::styled(" ────", Style::new().dim()),
                ])));
            }

            let msg = &rendered.msg;
            let mut lines = Vec::new();

            if rendered.group == MessageGroup::Full {
                let author = helpers::sanitize_for_display(&msg.author_display_name);
                let time = helpers::format_time_short(msg.timestamp);
                let delivery = match msg.delivery_status {
                    rekindle_types::display::DeliveryStatus::Sending => " ○",
                    rekindle_types::display::DeliveryStatus::Confirmed => " ●",
                    rekindle_types::display::DeliveryStatus::Failed => " ✗",
                };
                let delivery_style = match msg.delivery_status {
                    rekindle_types::display::DeliveryStatus::Failed => Style::new().bold(),
                    _ => Style::new().dim(),
                };
                lines.push(Line::from(vec![
                    Span::styled(author, Style::new().bold()),
                    Span::styled(delivery, delivery_style),
                    Span::raw("  "),
                    Span::styled(format!("[{time}]"), Style::new().dim()),
                ]));
            }

            if msg.is_encrypted {
                let hint = msg.needs_mek.map(|_| format!(
                    " — request: rekindle key mek request -c \"{}\" -C \"{}\"",
                    self.community, self.channel
                )).unwrap_or_default();
                lines.push(Line::from(vec![
                    Span::styled(format!("[encrypted, MEK gen {}]", msg.mek_generation), Style::new().dim().italic()),
                    Span::styled(hint, Style::new().dim()),
                ]));
            } else if rendered.has_patch_fence {
                // Inline patch rendering — uses cached detection to avoid re-scanning
                if let Some(diff_text) = crate::v2::patch::render::extract_patch_fence(&msg.body) {
                    let patch = crate::v2::patch::render::parse_diff_to_patch(&diff_text);
                    let patch_lines = crate::v2::patch::render::render_patch_lines(
                        &patch,
                        false,
                        Style::new().fg(ratatui::style::Color::Green),
                        Style::new().fg(ratatui::style::Color::Red),
                        Style::new().dim(),
                        Style::new().bold(),
                        Style::new().fg(ratatui::style::Color::Cyan),
                        Style::new().bold().dim(),
                    );
                    // Render any text before the patch fence
                    let fence_start = msg.body.find("```patch").unwrap_or(0);
                    if fence_start > 0 {
                        let preamble = helpers::sanitize_for_display(&msg.body[..fence_start]);
                        for line in preamble.trim().lines() {
                            lines.push(Line::from(format!("  {line}")));
                        }
                    }
                    lines.extend(patch_lines);
                    // Render any text after the closing fence
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
                    // has_patch_fence was true but extract failed (edge case after body edit)
                    let body = helpers::sanitize_for_display(&msg.body);
                    for line in body.lines() {
                        lines.push(highlight_file_paths(line));
                    }
                }
            } else {
                let body = helpers::sanitize_for_display(&msg.body);
                for line in body.lines() {
                    lines.push(highlight_file_paths(line));
                }
            }

            if let Some(reply_seq) = msg.reply_to_sequence {
                lines.push(Line::from(vec![
                    Span::styled("  ↳ reply to ", Style::new().dim()),
                    Span::styled(format!("#{reply_seq}"), Style::new().dim().italic()),
                ]));
            }

            // Pin indicator
            if rendered.pinned {
                lines.push(Line::from(Span::styled("  📌 pinned", Style::new().dim())));
            }

            // Reaction badges
            if let Some(ref reactions) = rendered.reactions {
                let mut reaction_spans: Vec<Span<'_>> = vec![Span::raw("  ")];
                for (emoji, count) in reactions {
                    reaction_spans.push(Span::styled(
                        format!("{emoji} {count}  "),
                        Style::new().dim(),
                    ));
                }
                lines.push(Line::from(reaction_spans));
            }

            // Thread indicator
            if rendered.thread_reply_count > 0 {
                let thread_label = if rendered.thread_reply_count == 1 {
                    "1 reply".to_string()
                } else {
                    format!("{} replies", rendered.thread_reply_count)
                };
                lines.push(Line::from(Span::styled(
                    format!("  🧵 {thread_label}"),
                    Style::new().dim(),
                )));
            }

            items.push(ListItem::new(lines));
        }
        items
    }

    /// Render the message list with cached items and auto-scroll indicator.
    pub fn draw_messages(&mut self, frame: &mut Frame, area: Rect) {
        if self.is_empty() {
            let block = Block::bordered()
                .title(format!(" #{} ", self.channel))
                .border_style(if self.is_focused { Style::new() } else { Style::new().dim() });
            frame.render_widget(Paragraph::new("No messages yet.").style(Style::new().dim()).block(block), area);
            return;
        }

        if self.generation != self.last_rendered_generation {
            self.cached_items = self.build_items();
            self.last_rendered_generation = self.generation;
        }

        let list = List::new(self.cached_items.clone())
            .block(
                Block::bordered()
                    .title(format!(" #{} ({} messages) ", self.channel, self.len()))
                    .border_style(if self.is_focused { Style::new() } else { Style::new().dim() }),
            )
            .highlight_style(Style::new().reversed());

        frame.render_stateful_widget(list, area, &mut self.list_state);

        if !self.auto_scroll && !self.is_empty() {
            let hint = Paragraph::new(" ↑ scrolled — press G to jump to latest ")
                .style(Style::new().dim().italic())
                .alignment(ratatui::layout::Alignment::Center);
            frame.render_widget(hint, Rect {
                x: area.x, y: area.bottom().saturating_sub(1),
                width: area.width, height: 1,
            });
        }
    }
}

/// Known file extensions for path detection in message bodies.
const FILE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "py", "go", "c", "cpp", "h", "hpp",
    "java", "kt", "rb", "ex", "exs", "erl", "hs", "ml", "mli", "scala",
    "swift", "m", "mm", "zig", "nim", "lua", "sh", "bash", "zsh", "fish",
    "toml", "yaml", "yml", "json", "xml", "html", "css", "scss", "md",
    "txt", "cfg", "conf", "ini", "env", "lock", "nix", "flake",
    "dockerfile", "makefile", "cmake",
];

/// Detect and highlight file paths in a message line.
fn highlight_file_paths(line: &str) -> Line<'static> {
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
                Style::new().fg(ratatui::style::Color::Cyan).underlined(),
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
