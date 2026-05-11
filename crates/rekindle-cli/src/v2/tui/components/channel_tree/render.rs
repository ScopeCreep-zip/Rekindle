//! Channel tree rendering — connectors, type indicators, unread badges.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem};
use ratatui::Frame;

use super::tree::ChannelTree;
use crate::v2::helpers::sanitize_for_display;
use crate::v2::tui::components::unread_badge;

impl ChannelTree {
    /// Build rendered list items from the flattened tree.
    pub fn build_items(&self) -> Vec<ListItem<'static>> {
        self.nodes.iter().enumerate().map(|(i, node)| {
            let pipe = if self.use_unicode { "│   " } else { "|   " };
            let indent = pipe.repeat(node.depth.saturating_sub(1) as usize);
            let connector = self.connector_for(i);

            let expand_marker = if node.has_children {
                if self.expanded.contains(&node.id) {
                    if self.use_unicode { "▾ " } else { "v " }
                } else if self.use_unicode { "▸ " } else { "> " }
            } else {
                "  "
            };

            let channel_prefix = if node.kind.is_empty() { "" } else {
                match node.kind.as_str() {
                    "voice" => if self.use_unicode { "🔊 " } else { "[V] " },
                    "announcement" => if self.use_unicode { "📢 " } else { "[A] " },
                    "forum" => if self.use_unicode { "📋 " } else { "[F] " },
                    "stage" => if self.use_unicode { "🎙 " } else { "[S] " },
                    "media" => if self.use_unicode { "🖼 " } else { "[M] " },
                    "events" => if self.use_unicode { "📅 " } else { "[E] " },
                    _ => "# ",
                }
            };

            let label = sanitize_for_display(&node.label);
            let unread_text = unread_badge::format_unread(node.unread);

            let mut spans = vec![
                Span::raw(format!("{indent}{connector}{expand_marker}{channel_prefix}")),
                Span::raw(label),
            ];
            if !unread_text.is_empty() {
                spans.push(Span::styled(format!(" {unread_text}"), Style::new().bold()));
            }

            ListItem::new(Line::from(spans))
        }).collect()
    }

    /// Determine the tree connector character for a node.
    fn connector_for(&self, index: usize) -> &str {
        let node = &self.nodes[index];
        if node.depth == 0 { return ""; }

        let is_last = self.nodes[index + 1..]
            .iter()
            .take_while(|n| n.depth >= node.depth)
            .all(|n| n.depth > node.depth);

        if is_last {
            if self.use_unicode { "└── " } else { "`-- " }
        } else if self.use_unicode { "├── " } else { "|-- " }
    }

    /// Render the channel tree as a bordered list.
    pub fn draw_tree(&mut self, frame: &mut Frame, area: Rect) {
        let items = self.build_items();
        let block = Block::bordered()
            .title(" Channels ")
            .border_style(if self.is_focused { Style::new() } else { Style::new().dim() });

        let list = List::new(items).block(block).highlight_style(Style::new().reversed());
        frame.render_stateful_widget(list, area, &mut self.list_state);
    }
}
