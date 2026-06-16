//! Voice session rendering — participant list with mute/deafen status.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

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
    channel: &str,
    caches: &mut RenderCaches,
) {
    let targets = caches.click_targets.entry(ViewKindTag::VoiceSession).or_default();
    targets.insert(FocusId::VoiceParticipants, area);

    let (session_community, session_channel) = state.voice.active_session.as_ref()
        .map(|s| (s.community.as_str(), s.channel.as_str()))
        .unwrap_or((community, channel));
    let name = state.communities.name_for(session_community);
    let block = Block::bordered()
        .title(format!(" Voice: {name} / #{session_channel} "))
        .border_style(theme.focused_border());

    match state.voice.active_session {
        Some(ref session) => {
            let count_label = format!("  Participants ({})", session.participants.len());
            let count_span = theme.span("accent", &count_label);
            let mut lines = vec![
                Line::from(vec![
                    theme.span("dim", "  Self: "),
                    Span::raw(if session.muted { "muted" } else { "unmuted" }),
                    Span::raw("  "),
                    Span::raw(if session.deafened { "deafened" } else { "listening" }),
                ]),
                Line::from(""),
                Line::from(count_span),
            ];
            for p in &session.participants {
                let mute_icon = if p.muted { "\u{1f507}" } else { "\u{1f50a}" };
                let deaf_icon = if p.deafened { " \u{1f6ab}" } else { "" };
                lines.push(Line::from(format!("    {mute_icon}{deaf_icon} {}", p.pseudonym)));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(theme.span("dim", "  [m] mute  [d] deafen  [q] leave")));
            frame.render_widget(Paragraph::new(lines).block(block), area);
        }
        None => {
            frame.render_widget(
                Paragraph::new("  No active voice session.").style(theme.style("dim")).block(block),
                area,
            );
        }
    }
}
