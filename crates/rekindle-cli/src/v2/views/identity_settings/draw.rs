//! Identity settings rendering — profile details, DHT records, security info.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
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
    caches: &mut RenderCaches,
) {
    let targets = caches.click_targets.entry(ViewKindTag::IdentitySettings).or_default();
    targets.insert(FocusId::IdentitySettings, area);

    let [content_area, help_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
    ]).areas(area);

    let (pk, dn) = state.ephemeral.identity.as_ref()
        .map(|id| (id.public_key.as_str(), id.display_name.as_str()))
        .unwrap_or(("", ""));

    let title = if dn.is_empty() {
        " Identity \u{2014} loading... ".to_string()
    } else {
        format!(" Identity \u{2014} {dn} ")
    };
    let block = Block::bordered().title(title).border_style(theme.focused_border());

    if state.ephemeral.identity.is_none() {
        frame.render_widget(
            Paragraph::new("  Loading identity...").style(theme.style("dim")).block(block),
            content_area,
        );
    } else {
        let snap = state.status_snapshot.as_ref();
        let route = snap.map_or("unknown".to_string(), |s| {
            if s.route_allocated {
                format!("allocated ({}s)", s.route_age_secs.unwrap_or(0))
            } else {
                "not allocated".into()
            }
        });
        let attachment = snap.map_or("unknown", |s| s.attachment.as_str());
        let watches = snap.map_or(0, |s| s.active_watches);
        let communities = snap.map_or(0, |s| s.community_count);
        let friends = snap.map_or(0, |s| s.friend_count);

        let lines = vec![
            kv_line("  Public Key", &helpers::abbreviate_key(pk), theme),
            kv_line("  Display Name", dn, theme),
            kv_line("  Attachment", attachment, theme),
            kv_line("  Route", &route, theme),
            kv_line("  Watches", &watches.to_string(), theme),
            kv_line("  Communities", &communities.to_string(), theme),
            kv_line("  Friends", &friends.to_string(), theme),
        ];

        frame.render_widget(Paragraph::new(lines).block(block), content_area);
    }

    frame.render_widget(Paragraph::new(Line::from(
        theme.span("dim", "  [y] yank key  [q] back"),
    )), help_area);
}

fn kv_line(key: &str, value: &str, theme: &ThemeManager) -> Line<'static> {
    let key_label = format!("{key:<18}");
    let key_span = theme.span("dim", &key_label);
    Line::from(vec![
        key_span,
        Span::raw(value.to_string()),
    ])
}
