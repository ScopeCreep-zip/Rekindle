//! File preview rendering — file content with line numbers, loaded from disk on cache miss.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::v2::tui::focus::FocusId;
use crate::v2::tui::state::navigation::ViewKindTag;
use crate::v2::tui::state::render_caches::{FilePreviewCache, RenderCaches};
use crate::v2::tui::state::TuiState;
use crate::v2::tui::theme::ThemeManager;

pub fn draw(
    state: &TuiState,
    frame: &mut Frame,
    area: Rect,
    theme: &ThemeManager,
    path: &str,
    line: Option<usize>,
    caches: &mut RenderCaches,
) {
    let targets = caches.click_targets.entry(ViewKindTag::FilePreview).or_default();
    targets.insert(FocusId::MessageList, area);

    let needs_load = caches.file_preview.as_ref().map_or(true, |c| c.path != path);
    if needs_load {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let line_count = content.lines().count();
                caches.file_preview = Some(FilePreviewCache {
                    path: path.to_string(),
                    content,
                    line_count,
                });
            }
            Err(e) => {
                let block = Block::bordered()
                    .title(format!(" {path} "))
                    .border_style(theme.focused_border());
                frame.render_widget(
                    Paragraph::new(format!("  Error reading file: {e}"))
                        .style(theme.style("error")).block(block),
                    area,
                );
                return;
            }
        }
    }

    let cache = caches.file_preview.as_ref().unwrap();
    let short_path = if path.len() > 60 {
        format!("...{}", &path[path.len() - 57..])
    } else {
        path.to_string()
    };
    let block = Block::bordered()
        .title(format!(" {short_path} ({} lines) ", cache.line_count))
        .border_style(theme.focused_border());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height as usize;
    let scroll_offset = state.ephemeral.file_preview_scroll_offset;
    let base = line.unwrap_or(0).saturating_sub(visible_height / 2) as i32;
    let computed = if scroll_offset == i32::MAX {
        cache.line_count.saturating_sub(visible_height)
    } else if scroll_offset == i32::MIN {
        0
    } else {
        (base + scroll_offset).max(0) as usize
    };
    let start_line = computed.min(cache.line_count.saturating_sub(visible_height));
    let lines: Vec<Line<'_>> = cache.content.lines()
        .enumerate()
        .skip(start_line)
        .take(visible_height)
        .map(|(i, text)| {
            let line_num = i + 1;
            let is_target = line.map_or(false, |l| l == line_num);
            let num_style = if is_target { theme.style("accent") } else { theme.style("dim") };
            Line::from(vec![
                Span::styled(format!("{line_num:>4} "), num_style),
                Span::raw(text),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}
