//! Dashboard rendering — 2x2 grid: Identity | Node / Communities | Friends.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};
use ratatui::Frame;

use crate::v2::helpers;
use crate::v2::tui::focus::FocusId;
use crate::v2::tui::state::navigation::ViewKindTag;
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;
use crate::v2::tui::theme::ThemeManager;
use crate::v2::tui::widgets::sparkline_inline::InlineSparkline;

pub fn draw(
    state: &TuiState,
    frame: &mut Frame,
    area: Rect,
    theme: &ThemeManager,
    caches: &mut RenderCaches,
) {
    if state.ephemeral.spinner.is_active() {
        let block = Block::bordered().title(" Dashboard ").border_style(theme.focused_border());
        let glyph = state.ephemeral.spinner.glyph();
        let label = &state.ephemeral.spinner.label;
        frame.render_widget(
            Paragraph::new(format!("  {glyph} {label}")).style(theme.style("dim")).block(block),
            area,
        );
        return;
    }

    let [top_row, bottom_row] = Layout::vertical([
        Constraint::Length(9),
        Constraint::Fill(1),
    ]).areas(area);

    let [identity_area, node_area] = Layout::horizontal([
        Constraint::Percentage(50),
        Constraint::Percentage(50),
    ]).areas(top_row);

    let [communities_area, friends_area] = Layout::horizontal([
        Constraint::Percentage(50),
        Constraint::Percentage(50),
    ]).areas(bottom_row);

    let targets = caches.click_targets.entry(ViewKindTag::Dashboard).or_default();
    targets.insert(FocusId::DashIdentity, identity_area);
    targets.insert(FocusId::DashNode, node_area);
    targets.insert(FocusId::ChannelTree, communities_area);
    targets.insert(FocusId::FriendList, friends_area);

    render_identity(state, frame, identity_area, theme);
    render_node(state, frame, node_area, theme, caches);
    render_communities(state, frame, communities_area, theme);
    render_friends(state, frame, friends_area, theme);
}

fn render_identity(state: &TuiState, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
    let (pk, dn) = state.ephemeral.identity.as_ref()
        .map(|id| (id.public_key.as_str(), id.display_name.as_str()))
        .unwrap_or(("", ""));
    let key_short = helpers::abbreviate_key(pk);

    let lines = vec![
        Line::from(vec![
            theme.span("dim", "  Public key:    "),
            Span::raw(key_short),
        ]),
        Line::from(vec![
            theme.span("dim", "  Display name:  "),
            theme.span("accent", dn),
        ]),
    ];

    let focused = state.nav.focus_ring.is_focused(FocusId::DashIdentity);
    let border = if focused { theme.focused_border() } else { theme.unfocused_border() };
    frame.render_widget(Paragraph::new(lines).block(Block::bordered().title(" Identity ").border_style(border)), area);
}

fn render_node(state: &TuiState, frame: &mut Frame, area: Rect, theme: &ThemeManager, caches: &mut RenderCaches) {
    let focused = state.nav.focus_ring.is_focused(FocusId::DashNode);
    let border = if focused { theme.focused_border() } else { theme.unfocused_border() };
    let block = Block::bordered().title(" Node ").border_style(border);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let (status_glyph, status_label, _) = theme.presence_indicator(
        if state.node_connected { "online" } else { "offline" },
    );

    let snap = state.status_snapshot.as_ref();
    let uptime = snap.map_or(0, |s| s.uptime_secs);
    let route_age = snap.and_then(|s| s.route_age_secs).unwrap_or(0);
    let active_watches = snap.map_or(0, |s| s.active_watches);
    let community_count = snap.map_or(0, |s| s.community_count);

    let route_status = if route_age == 0 {
        "no route allocated".to_string()
    } else if route_age < 60 {
        format!("route healthy ({route_age}s)")
    } else if route_age < 300 {
        format!("route aging ({route_age}s)")
    } else {
        format!("route stale ({route_age}s)")
    };

    let watch_status = if community_count == 0 {
        "no communities".to_string()
    } else {
        let expected = community_count * 3;
        format!("{active_watches} of {expected} DHT watches active")
    };

    let lines = vec![
        Line::from(vec![
            theme.span("dim", "  Status: "),
            Span::raw(format!("{status_glyph} {status_label}")),
        ]),
        Line::from(vec![
            theme.span("dim", "  Peers:  "),
            Span::raw(format!("{} connected", state.cached_peer_count)),
        ]),
        Line::from(vec![
            theme.span("dim", "  Uptime: "),
            Span::raw(helpers::format_uptime(uptime)),
        ]),
        Line::from(vec![
            theme.span("dim", "  Route:  "),
            Span::raw(route_status),
        ]),
        Line::from(vec![
            theme.span("dim", "  Watch:  "),
            Span::raw(watch_status),
        ]),
    ];

    let meter_width = inner.width / 3;
    let [text_area, meter_area] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(meter_width),
    ]).areas(inner);

    frame.render_widget(Paragraph::new(lines), text_area);

    if meter_area.width >= 10 {
        // Peer sparkline (braille trend line)
        if !state.ephemeral.peer_history.is_empty() {
            let data: Vec<f64> = state.ephemeral.peer_history.iter().copied().collect();
            let sparkline = InlineSparkline {
                data: &data,
                gradient: theme.gradient_cpu(),
                max_value: 0.0,
            };
            (&sparkline).render(Rect { height: 1, ..meter_area }, frame.buffer_mut());
        }

        // Compute meter values
        let route_health = if route_age == 0 { 0u8 }
            else if route_age < 60 { 100 }
            else if route_age < 300 { 70 }
            else if route_age < 600 { 40 }
            else { 10 };

        let watch_pct = if community_count > 0 {
            ((active_watches as u32 * 100) / (community_count as u32 * 3).max(1)).min(100) as u8
        } else { 0 };

        let net_activity = if state.ephemeral.peer_history.len() >= 2 {
            let recent = state.ephemeral.peer_history.back().copied().unwrap_or(0.0);
            let prev = state.ephemeral.peer_history.iter().rev().nth(1).copied().unwrap_or(0.0);
            let delta = (recent - prev).abs();
            (delta * 10.0).min(100.0) as u8
        } else { 0 };

        let mem_free_pct = 100u8.saturating_sub(watch_pct);

        // Labeled meters: [LBL ████████ NNN%]
        let label_width = 6u16;
        let value_width = 5u16;
        let bar_width = meter_area.width.saturating_sub(label_width + value_width);
        let use_unicode = theme.use_unicode();

        // Route health via CachedMeter (persists across frames, amortized gradient lookup)
        let route_y = meter_area.y + 1;
        if route_y < inner.bottom() && bar_width >= 3 {
            let route_label = theme.span("dim", "Route");
            frame.render_widget(Paragraph::new(route_label), Rect { x: meter_area.x, y: route_y, width: label_width, height: 1 });

            let needs_rebuild = caches.route_health_meter.as_ref()
                .map_or(true, |c| c.width() != bar_width || c.unicode() != use_unicode);
            if needs_rebuild {
                caches.route_health_meter = Some(crate::v2::tui::widgets::meter::CachedMeter::new(
                    bar_width,
                    theme.gradient_temp().clone(),
                    theme.color("meter.bg"),
                    use_unicode,
                ));
            }
            let cached = caches.route_health_meter.as_mut().unwrap();
            cached.render_at(route_health, meter_area.x + label_width, route_y, frame.buffer_mut());

            let route_val = if route_age == 0 { "  --".to_string() } else { format!("{:>3}s", route_age.min(999)) };
            let route_val_span = theme.span("dim", &route_val);
            frame.render_widget(Paragraph::new(route_val_span), Rect { x: meter_area.x + label_width + bar_width, y: route_y, width: value_width, height: 1 });
        }

        // Remaining meters via GradientMeter with actual values
        let watch_val = if community_count > 0 {
            let expected = community_count * 3;
            format!("{}/{}", active_watches, expected)
        } else {
            "  --".into()
        };
        let peers_now = state.ephemeral.peer_history.back().copied().unwrap_or(0.0) as u32;
        let peers_prev = state.ephemeral.peer_history.iter().rev().nth(1).copied().unwrap_or(0.0) as u32;
        let peer_delta = peers_now as i32 - peers_prev as i32;
        let net_dl_val = if peer_delta != 0 || peers_now > 0 {
            format!("{:>2}\u{0394}", peer_delta.abs())
        } else {
            "  --".into()
        };
        let net_ul_val = net_dl_val.clone();
        let free_val = if community_count > 0 {
            let capacity = community_count * 3;
            let free = capacity.saturating_sub(active_watches);
            format!("{}/{}", free, capacity)
        } else {
            "  --".into()
        };

        let remaining_meters: Vec<(&str, u8, &crate::v2::tui::palette::Gradient, String)> = vec![
            ("Watch", watch_pct, theme.gradient_mem_used(), watch_val),
            ("Net \u{2193}", net_activity, theme.gradient_net_download(), net_dl_val),
            ("Net \u{2191}", net_activity, theme.gradient_net_upload(), net_ul_val),
            ("Free ", mem_free_pct, theme.gradient_mem_free(), free_val),
        ];

        for (row, (label, value, gradient, display)) in remaining_meters.iter().enumerate() {
            let y = meter_area.y + 2 + row as u16;
            if y >= inner.bottom() { break; }

            let label_span = theme.span("dim", label);
            frame.render_widget(Paragraph::new(label_span), Rect { x: meter_area.x, y, width: label_width, height: 1 });

            if bar_width >= 3 {
                let bar_rect = Rect { x: meter_area.x + label_width, y, width: bar_width, height: 1 };
                let meter_widget = crate::v2::tui::widgets::meter::GradientMeter {
                    value: *value,
                    gradient,
                    bg_color: theme.color("meter.bg"),
                    invert: false,
                    unicode: use_unicode,
                };
                (&meter_widget).render(bar_rect, frame.buffer_mut());
            }

            let value_text = format!("{:>5}", display);
            let value_span = theme.span("dim", &value_text);
            frame.render_widget(Paragraph::new(value_span), Rect { x: meter_area.x + label_width + bar_width, y, width: value_width, height: 1 });
        }
    }
}

fn render_communities(state: &TuiState, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
    let focused = state.nav.focus_ring.is_focused(FocusId::ChannelTree);
    let border = if focused { theme.focused_border() } else { theme.unfocused_border() };
    let title = format!(" Communities ({}) ", state.communities.list.len());
    let block = Block::bordered().title(title).border_style(border);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.communities.list.is_empty() {
        frame.render_widget(
            Paragraph::new("  No communities joined.\n  Join one: rekindle community join --invite <code>")
                .style(theme.style("dim")),
            inner,
        );
        return;
    }

    let [list_area, spark_area] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(inner.width / 4),
    ]).areas(inner);

    let lines: Vec<Line<'_>> = state.communities.list.iter().map(|c| {
        let pseudo = if c.pseudonym.is_empty() { String::new() } else { format!("  as {}", c.pseudonym) };
        Line::from(vec![
            Span::raw("  "),
            theme.span("accent", &c.name),
            theme.span("dim", &pseudo),
            theme.span("dim", if c.is_operator { "  [operator]" } else { "" }),
        ])
    }).collect();

    frame.render_widget(Paragraph::new(lines), list_area);

    // Community count sparkline with custom gradient
    if !state.ephemeral.community_history.is_empty() && spark_area.width >= 5 {
        let data: Vec<f64> = state.ephemeral.community_history.iter().copied().collect();
        let gradient = theme.custom_gradient_two("sapphire", "mauve");
        let sparkline = InlineSparkline {
            data: &data,
            gradient: &gradient,
            max_value: 0.0,
        };
        (&sparkline).render(Rect { height: 1, ..spark_area }, frame.buffer_mut());
    }

    // Community health gradient meter (3-color)
    if spark_area.width >= 5 && !state.communities.list.is_empty() {
        let health = (state.communities.list.len() * 10).min(100) as u8;
        let health_gradient = theme.custom_gradient_three("green", "yellow", "red");
        let health_meter = crate::v2::tui::widgets::meter::GradientMeter {
            value: health,
            gradient: &health_gradient,
            bg_color: theme.color("meter.bg"),
            invert: false,
            unicode: theme.use_unicode(),
        };
        let meter_row = Rect { y: spark_area.y + 1, height: 1, ..spark_area };
        if meter_row.y < inner.bottom() {
            (&health_meter).render(meter_row, frame.buffer_mut());
        }
    }

    // Process activity sparkline
    if !state.ephemeral.peer_history.is_empty() && spark_area.width >= 5 {
        let data: Vec<f64> = state.ephemeral.peer_history.iter().copied().collect();
        let sparkline = InlineSparkline {
            data: &data,
            gradient: theme.gradient_process(),
            max_value: 0.0,
        };
        let spark_row_2 = Rect { y: spark_area.y + 2, height: 1, ..spark_area };
        if spark_row_2.y < inner.bottom() {
            (&sparkline).render(spark_row_2, frame.buffer_mut());
        }
    }
}

fn render_friends(state: &TuiState, frame: &mut Frame, area: Rect, theme: &ThemeManager) {
    let online = state.friends.friends.iter().filter(|f| f.status == "online").count();
    let away = state.friends.friends.iter().filter(|f| f.status == "away").count();
    let offline = state.friends.friends.iter().filter(|f| f.status == "offline").count();
    let total = state.friends.friends.len();

    let focused = state.nav.focus_ring.is_focused(FocusId::FriendList);
    let border = if focused { theme.focused_border() } else { theme.unfocused_border() };
    let block = Block::bordered().title(format!(" Friends ({total}) ")).border_style(border);

    if state.friends.friends.is_empty() {
        frame.render_widget(
            Paragraph::new("  No friends yet.\n  Add one: rekindle friend add --target <key>")
                .style(theme.style("dim")).block(block),
            area,
        );
        return;
    }

    let rest = format!("  {away} away  {offline} offline");
    let summary = Line::from(vec![
        Span::raw("  "),
        Span::styled(format!("{online} online"), theme.style("accent").add_modifier(Modifier::BOLD)),
        theme.span("dim", &rest),
    ]);

    frame.render_widget(Paragraph::new(vec![summary]).block(block), area);
}
