//! Dashboard rendering — 2x2 grid: Identity | Node / Communities | Friends.

use anyhow::Result;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};
use ratatui::Frame;

use super::state::DashboardView;
use crate::v2::helpers;
use crate::v2::tui::focus::{FocusId, FocusRing};
use crate::v2::tui::theme::ThemeManager;
use crate::v2::tui::widgets::sparkline_inline::InlineSparkline;
use crate::v2::tui::widgets::meter::GradientMeter;
use crate::v2::tui::widgets::braille_graph::BrailleGraph;
use crate::v2::tui::widgets::colored_area::ColoredAreaGraph;
use crate::v2::views::View;

impl DashboardView {
    pub(super) fn render_identity(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        let key_short = helpers::abbreviate_key(&self.identity_public_key);
        let lines = vec![
            Line::from(vec![
                Span::styled("  Public key:    ", theme.style("dim")),
                Span::raw(key_short),
            ]),
            Line::from(vec![
                Span::styled("  Display name:  ", theme.style("dim")),
                Span::styled(self.identity_display_name.clone(), Style::new().bold()),
            ]),
        ];

        let border = if self.focus.is_focused(FocusId::DashIdentity) { theme.focused_border() } else { theme.unfocused_border() };
        frame.render_widget(Paragraph::new(lines).block(Block::bordered().title(" Identity ").border_style(border)), area);
    }

    pub(super) fn render_node(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        let border = if self.focus.is_focused(FocusId::DashNode) { theme.focused_border() } else { theme.unfocused_border() };
        let block = Block::bordered().title(" Node ").border_style(border);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.height < 3 || inner.width < 20 {
            // Too small for the full layout — fall back to compact text
            let (glyph, label, _color) = theme.presence_indicator(if self.node_attached { "online" } else { "offline" });
            frame.render_widget(
                Paragraph::new(format!("  {glyph} {label}  {} peers", self.node_peer_count)),
                inner,
            );
            return;
        }

        let [text_area, sparkline_area] = Layout::horizontal([
            Constraint::Fill(1), Constraint::Length(inner.width / 3),
        ]).areas(inner);

        // Left side: text status
        let (status_glyph, status_label, _status_color) = theme.presence_indicator(
            if self.node_attached { "online" } else { "offline" }
        );
        let internet = if self.node_public_internet { theme.status_glyph(true) } else { theme.status_glyph(false) };
        let route = if self.node_route_allocated { theme.status_glyph(true) } else { theme.status_glyph(false) };

        let mut lines = vec![
            Line::from(vec![
                Span::styled("  Status: ", theme.style("dim")),
                Span::raw(format!("{status_glyph} {status_label}")),
            ]),
            Line::from(vec![
                Span::styled("  Peers:  ", theme.style("dim")),
                Span::raw(self.node_peer_count.to_string()),
                Span::styled(format!("  Internet: {internet}  Route: {route}"), theme.style("dim")),
            ]),
            Line::from(vec![
                Span::styled("  Uptime: ", theme.style("dim")),
                Span::raw(helpers::format_uptime(self.node_uptime_secs)),
            ]),
        ];

        if self.active_transfers > 0 {
            lines.push(Line::from(vec![
                Span::styled("  Transfers: ", theme.style("dim")),
                Span::styled(format!("{} active", self.active_transfers), Style::new().bold()),
            ]));
        } else if self.bytes_sent > 0 || self.bytes_received > 0 {
            let sent = helpers::format_bytes(self.bytes_sent);
            let recv = helpers::format_bytes(self.bytes_received);
            lines.push(Line::from(vec![
                Span::styled("  Data: ", theme.style("dim")),
                Span::raw(format!("↑{sent} ↓{recv}")),
            ]));
        }
        frame.render_widget(Paragraph::new(lines), text_area);

        // Right side: peer count visualization (history over time)
        if !self.peer_history.is_empty() && sparkline_area.width >= 5 {
            if sparkline_area.height >= 3 {
                // Tall enough for colored area graph (double vertical resolution)
                let area_graph = ColoredAreaGraph {
                    data: &self.peer_history,
                    gradient: theme.gradient_cpu(),
                };
                let graph_rows = Rect { height: sparkline_area.height.saturating_sub(1), ..sparkline_area };
                (&area_graph).render(graph_rows, frame.buffer_mut());

                // Uptime meter on the bottom row (% of 24h)
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, clippy::cast_precision_loss)]
                let uptime_pct = ((self.node_uptime_secs as f64 / 86400.0) * 100.0).min(100.0) as u8;
                let meter = GradientMeter {
                    value: uptime_pct,
                    gradient: theme.gradient_temp(),
                    bg_color: theme.color("meter.bg"),
                    invert: false,
                };
                let meter_row = Rect { y: sparkline_area.bottom().saturating_sub(1), height: 1, ..sparkline_area };
                (&meter).render(meter_row, frame.buffer_mut());
            } else {
                // Short — use inline sparkline (single row)
                let data: Vec<f64> = self.peer_history.iter().copied().collect();
                let sparkline = InlineSparkline {
                    data: &data,
                    gradient: theme.gradient_cpu(),
                    max_value: 0.0,
                };
                let spark_row = Rect { height: 1, ..sparkline_area };
                (&sparkline).render(spark_row, frame.buffer_mut());
            }
        }
    }

    pub(super) fn render_communities(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        let title = format!(" Communities ({}) ", self.communities.len());
        let border = if self.focus.is_focused(FocusId::ChannelTree) { theme.focused_border() } else { theme.unfocused_border() };
        let block = Block::bordered().title(title).border_style(border);

        if self.communities.is_empty() {
            frame.render_widget(
                Paragraph::new("  No communities joined.\n  Join one: rekindle community join --invite <code>")
                    .style(theme.style("dim")).block(block),
                area,
            );
            return;
        }

        let lines: Vec<Line<'_>> = self.communities.iter().map(|c| {
            Line::from(vec![
                Span::raw("  "),
                Span::styled(&c.name, Style::new().bold()),
                Span::styled(format!("  {} members  {} channels", c.member_count, c.channel_count), theme.style("dim")),
            ])
        }).collect();

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Split: text list left, activity graph right
        if !self.community_history.is_empty() && inner.width >= 30 {
            let [text_area, graph_area] = Layout::horizontal([
                Constraint::Fill(1), Constraint::Length(inner.width / 4),
            ]).areas(inner);
            frame.render_widget(Paragraph::new(lines), text_area);

            // Braille graph of community count over time using a custom gradient
            let custom_grad = theme.custom_gradient_two("teal", "lavender");
            let graph = BrailleGraph {
                data: &self.community_history,
                gradient: &custom_grad,
                ghost_fg: theme.color("meter.bg"),
                show_ghost: true,
            };
            (&graph).render(graph_area, frame.buffer_mut());
        } else {
            frame.render_widget(Paragraph::new(lines), inner);
        }
    }

    pub(super) fn render_friends(&self, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
        let online = self.friends.iter().filter(|f| f.status == "online").count();
        let away = self.friends.iter().filter(|f| f.status == "away").count();
        let offline = self.friends.iter().filter(|f| f.status == "offline").count();
        let total = self.friends.len();

        let border = if self.focus.is_focused(FocusId::FriendList) { theme.focused_border() } else { theme.unfocused_border() };
        let block = Block::bordered().title(format!(" Friends ({total}) ")).border_style(border);

        if self.friends.is_empty() {
            frame.render_widget(
                Paragraph::new("  No friends yet.\n  Add one: rekindle friend add --target <key>")
                    .style(theme.style("dim")).block(block),
                area,
            );
            return;
        }

        let summary = Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{online} online"), Style::new().bold()),
            Span::styled(format!("  {away} away  {offline} offline"), theme.style("dim")),
        ]);

        let preview_count = 6.min(self.friends.len());
        let mut previews: Vec<Span<'_>> = vec![Span::raw("  ")];
        for f in self.friends.iter().take(preview_count) {
            let glyph = match f.status.as_str() {
                "online" => if self.use_unicode { "● " } else { "o " },
                "away" => if self.use_unicode { "◐ " } else { "~ " },
                _ => if self.use_unicode { "○ " } else { ". " },
            };
            previews.push(Span::raw(format!("{glyph}{}", helpers::sanitize_for_display(&f.display_name))));
            previews.push(Span::raw("  "));
        }
        if self.friends.len() > preview_count {
            previews.push(Span::styled(format!("...+{} more", self.friends.len() - preview_count), theme.style("dim")));
        }

        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(Paragraph::new(vec![summary, Line::from(previews)]), Rect { height: inner.height.saturating_sub(1), ..inner });

        // Online ratio meter at bottom of friends panel (green → yellow → red)
        if total > 0 && inner.height >= 3 {
            #[allow(clippy::cast_possible_truncation)]
            let online_pct = (online * 100 / total) as u8;
            let presence_gradient = theme.custom_gradient_three("green", "yellow", "red");
            let meter = GradientMeter {
                value: online_pct,
                gradient: &presence_gradient,
                bg_color: theme.color("meter.bg"),
                invert: true, // high = green (left), low = red (right)
            };
            let meter_row = Rect { y: inner.bottom().saturating_sub(1), height: 1, ..inner };
            (&meter).render(meter_row, frame.buffer_mut());
        }
    }
}

impl crate::v2::views::ViewQuery for DashboardView {}

impl View for DashboardView {
    fn draw(&mut self, frame: &mut Frame, area: Rect, theme: &ThemeManager) -> Result<()> {
        if !self.loaded {
            let block = Block::bordered().title(" Dashboard ").border_style(theme.focused_border());
            frame.render_widget(Paragraph::new("  Loading dashboard data...").style(theme.style("dim")).block(block), area);
            return Ok(());
        }

        let [top_row, bottom_row] = Layout::vertical([Constraint::Length(7), Constraint::Fill(1)]).areas(area);
        let [identity_area, node_area] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(top_row);
        let [communities_area, friends_area] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(bottom_row);

        self.panel_rects = [identity_area, node_area, communities_area, friends_area];

        self.render_identity(frame, identity_area, theme);
        self.render_node(frame, node_area, theme);
        self.render_communities(frame, communities_area, theme);
        self.render_friends(frame, friends_area, theme);
        Ok(())
    }

    fn update(&mut self, action: crate::v2::tui::action::Action) -> Result<Option<crate::v2::tui::action::Action>> {
        Ok(super::input::handle_update(self, &action))
    }

    fn on_command_result(&mut self, result: crate::v2::tui::action::CommandResult) -> Result<()> {
        super::events::handle_command_result(self, result);
        Ok(())
    }

    fn on_subscription_event(&mut self, event: &rekindle_types::subscription_events::SubscriptionEvent) -> Result<()> {
        super::events::handle_subscription_event(self, event);
        Ok(())
    }

    fn handle_focused_key(&mut self, key: crossterm::event::KeyEvent) -> Option<crate::v2::tui::action::Action> {
        super::input::handle_focused_key(self, key)
    }

    fn handle_click(&mut self, column: u16, row: u16) -> Option<crate::v2::tui::action::Action> {
        super::input::handle_click(self, column, row)
    }

    fn focus_ring(&mut self) -> &mut FocusRing { &mut self.focus }
}
