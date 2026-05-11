//! Project-wide search commands: fuzzy file search and content grep.
//!
//! Uses fff via `RekindleSearch::init_oneshot()` for synchronous CLI usage.
//! No daemon connection needed — searches the local filesystem directly.

use crate::v2::output::format;
use crate::v2::output::OutputMode;
use crate::v2::search::RekindleSearch;

/// `rekindle search <query>` — fuzzy file search.
pub fn cmd_search(query: &str, limit: usize, mode: OutputMode) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()
        .map_or_else(|_| ".".into(), |p| p.to_string_lossy().to_string());

    let search = RekindleSearch::init_oneshot(&cwd)?;
    let results = search.search_files(query, None, limit);

    if mode.is_structured() {
        let items: Vec<serde_json::Value> = results.iter().map(|(path, score)| {
            serde_json::json!({
                "path": path,
                "score": score.total,
                "match_type": score.match_type,
                "exact": score.exact_match,
            })
        }).collect();
        return format::print_structured(&items, mode);
    }

    if results.is_empty() {
        return format::print_text("No matches.");
    }

    for (path, score) in &results {
        let mut stdout = std::io::stdout().lock();
        use std::io::Write;
        writeln!(stdout, "{path}\t(score: {})", score.total)?;
    }
    Ok(())
}

/// `rekindle grep <query>` — content grep.
pub fn cmd_grep(
    query: &str,
    regex_mode: bool,
    limit: usize,
    before: usize,
    after: usize,
    mode: OutputMode,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()
        .map_or_else(|_| ".".into(), |p| p.to_string_lossy().to_string());

    let grep_mode = if regex_mode {
        fff_search::GrepMode::Regex
    } else {
        fff_search::GrepMode::PlainText
    };

    // Create picker directly with content indexing for grep (no intermediate init_oneshot)
    let mut picker = fff_search::file_picker::FilePicker::new(fff_search::FilePickerOptions {
        base_path: cwd,
        enable_mmap_cache: false,
        enable_content_indexing: true,
        watch: false,
        mode: fff_search::FFFMode::Neovim,
        cache_budget: None,
    })?;
    picker.collect_files()?;

    let parsed = fff_search::grep::parse_grep_query(query);
    let result = picker.grep(&parsed, &fff_search::GrepSearchOptions {
        mode: grep_mode,
        smart_case: true,
        page_limit: limit,
        before_context: before,
        after_context: after,
        classify_definitions: true,
        ..Default::default()
    });

    if mode.is_structured() {
        let items: Vec<serde_json::Value> = result.matches.iter().map(|m| {
            let file_path = result.files.get(m.file_index)
                .map(|f| f.relative_path(&picker))
                .unwrap_or_default();
            serde_json::json!({
                "file": file_path,
                "line": m.line_number,
                "col": m.col,
                "content": m.line_content,
                "is_definition": m.is_definition,
                "context_before": m.context_before,
                "context_after": m.context_after,
            })
        }).collect();
        return format::print_structured(&items, mode);
    }

    if result.matches.is_empty() {
        return format::print_text("No matches.");
    }

    let mut stdout = std::io::stdout().lock();
    use std::io::Write;
    for m in &result.matches {
        let file_path = result.files.get(m.file_index)
            .map(|f| f.relative_path(&picker))
            .unwrap_or_default();

        for ctx in &m.context_before {
            writeln!(stdout, "  {ctx}")?;
        }

        let def_marker = if m.is_definition { " [def]" } else { "" };
        writeln!(stdout, "{file_path}:{}: {}{def_marker}", m.line_number, m.line_content)?;

        for ctx in &m.context_after {
            writeln!(stdout, "  {ctx}")?;
        }
    }

    writeln!(stdout, "\n{} matches in {} files ({} searched)",
        result.matches.len(), result.files_with_matches, result.total_files_searched)?;
    Ok(())
}
