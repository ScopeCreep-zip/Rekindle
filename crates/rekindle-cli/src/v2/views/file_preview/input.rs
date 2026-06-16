//! File preview input — scroll, copy path, jump to target line.

use crossterm::event::{KeyCode, MouseButton, MouseEventKind};

use crate::v2::tui::effects::Effect;
use crate::v2::tui::events::TerminalEvent;
use crate::v2::tui::state::navigation::ViewKindTag;
use crate::v2::tui::state::render_caches::RenderCaches;
use crate::v2::tui::state::TuiState;

pub fn handle(
    event: &TerminalEvent,
    state: &mut TuiState,
    path: &str,
    line: Option<usize>,
    caches: &mut RenderCaches,
) -> Vec<Effect> {
    match event {
        TerminalEvent::Key(key) => {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    state.ephemeral.file_preview_scroll_offset += 1;
                    vec![]
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    state.ephemeral.file_preview_scroll_offset -= 1;
                    vec![]
                }
                KeyCode::Char('G') | KeyCode::End => {
                    state.ephemeral.file_preview_scroll_offset = i32::MAX;
                    vec![]
                }
                KeyCode::Home => {
                    state.ephemeral.file_preview_scroll_offset = if line.is_some() { 0 } else { i32::MIN };
                    vec![]
                }
                KeyCode::PageDown => {
                    state.ephemeral.file_preview_scroll_offset += 20;
                    vec![]
                }
                KeyCode::PageUp => {
                    state.ephemeral.file_preview_scroll_offset -= 20;
                    vec![]
                }
                KeyCode::Char('g') => {
                    state.ephemeral.file_preview_scroll_offset = 0;
                    vec![]
                }
                KeyCode::Char('y') => {
                    vec![Effect::SetClipboard(path.to_string())]
                }
                _ => vec![],
            }
        }
        TerminalEvent::Mouse(mouse) => {
            if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                if let Some(targets) = caches.click_targets.get(&ViewKindTag::FilePreview) {
                    for (&focus_id, rect) in targets {
                        if mouse.column >= rect.x && mouse.column < rect.x + rect.width
                            && mouse.row >= rect.y && mouse.row < rect.y + rect.height
                        {
                            state.nav.focus_ring.set(focus_id);
                            return vec![];
                        }
                    }
                }
            }
            vec![]
        }
        _ => vec![],
    }
}
