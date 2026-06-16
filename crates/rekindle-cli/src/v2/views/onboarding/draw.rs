//! Onboarding wizard rendering — welcome screen and question steps.

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
    caches: &mut RenderCaches,
) {
    let targets = caches.click_targets.entry(ViewKindTag::Onboarding).or_default();
    targets.insert(FocusId::MessageList, area);

    let name = state.communities.name_for(community);
    let block = Block::bordered()
        .title(format!(" Welcome to {name} "))
        .border_style(theme.focused_border());

    let onboarding = state.communities.onboarding.get(community);
    match onboarding {
        None => {
            frame.render_widget(
                Paragraph::new("  Loading onboarding...").style(theme.style("dim")).block(block),
                area,
            );
        }
        Some((config, welcome)) => {
            let mut lines = Vec::new();

            if let Some(ref w) = welcome {
                lines.push(Line::from(theme.span("accent", &w.title)));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::raw(&w.body)));
                lines.push(Line::from(""));
                for rule in &w.rules {
                    lines.push(Line::from(format!("  \u{2022} {rule}")));
                }
                lines.push(Line::from(""));
            }

            if let Some(ref c) = config {
                if c.enabled {
                    let current = state.ephemeral.onboarding_step;
                    let total = c.steps.len();
                    if let Some(step) = c.steps.get(current) {
                        lines.push(Line::from(Span::styled(
                            format!("Step {} of {}: {}", current + 1, total, step.title),
                            theme.style("accent"),
                        )));
                        lines.push(Line::from(theme.span("dim", &step.description)));
                        lines.push(Line::from(""));
                        for q in &step.questions {
                            let required = if q.required { " *" } else { "" };
                            lines.push(Line::from(format!("    {}{required}", q.prompt)));
                        }
                        lines.push(Line::from(""));
                        let nav_hint = if current == 0 && total > 1 {
                            "  [n/Enter] next step"
                        } else if current + 1 >= total {
                            "  [p/Backspace] previous"
                        } else {
                            "  [p/Backspace] previous  [n/Enter] next"
                        };
                        lines.push(Line::from(theme.span("dim", nav_hint)));
                    } else {
                        lines.push(Line::from("  No steps configured."));
                    }
                } else {
                    lines.push(Line::from("  Onboarding is not enabled for this community."));
                }
            }

            if lines.is_empty() {
                lines.push(Line::from("  No onboarding configured."));
            }

            frame.render_widget(Paragraph::new(lines).block(block), area);
        }
    }
}
