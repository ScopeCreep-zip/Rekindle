//! Doctor input — scroll, refresh.

use crossterm::event::KeyCode;

use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::TerminalEvent;
use crate::v2::tui::state::navigation::ViewKind;
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;

pub fn handle(
    event: &TerminalEvent,
    state: &mut TuiState,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    let TerminalEvent::Key(key) = event else { return vec![]; };

    match key.code {
        KeyCode::Char('h') => {
            state.nav.pop_view();
            vec![]
        }
        KeyCode::Char('j') | KeyCode::Down => {
            caches.doctor_list.select_next();
            vec![]
        }
        KeyCode::Char('k') | KeyCode::Up => {
            caches.doctor_list.select_previous();
            vec![]
        }
        KeyCode::Char('G') | KeyCode::End => {
            caches.doctor_list.select_last();
            vec![]
        }
        KeyCode::Home => {
            caches.doctor_list.select_first();
            vec![]
        }
        KeyCode::Char('r') => {
            state.ephemeral.toasts.push(
                "Refreshing diagnostics...".into(),
                crate::v2::tui::state::ephemeral::ToastLevel::Info,
                state.now,
            );
            vec![Effect::Navigate(ViewKind::Doctor)]
        }
        KeyCode::Char('y') => {
            // Yank selected check value to clipboard
            if let Some(snap) = state.status_snapshot.as_ref() {
                if let Some(idx) = caches.doctor_list.selected() {
                    // Build items to map visual index to check — skip category headers
                    let mut check_idx = 0usize;
                    let mut current_cat: Option<&str> = None;
                    for check in &snap.checks {
                        if current_cat != Some(check.category.as_str()) {
                            current_cat = Some(check.category.as_str());
                            if check_idx == idx {
                                // Selected a category header — yank category name
                                return vec![Effect::SetClipboard(check.category.clone())];
                            }
                            check_idx += 1;
                        }
                        if check_idx == idx {
                            return vec![Effect::SetClipboard(format!("{}: {}", check.id, check.value))];
                        }
                        check_idx += 1;
                        // Skip description lines
                        if check.status != rekindle_types::display::CheckStatus::Pass && !check.description.is_empty() {
                            for _ in check.description.lines() {
                                if check_idx == idx {
                                    return vec![Effect::SetClipboard(check.description.clone())];
                                }
                                check_idx += 1;
                            }
                        }
                    }
                }
            }
            vec![]
        }
        _ => vec![],
    }
}
