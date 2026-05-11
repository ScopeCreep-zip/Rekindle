//! Patch rendering — produce styled ratatui Lines from unified diff text.
//!
//! Renders diffs inline in the message list with:
//! - Green/red background tinting for added/removed lines
//! - Dim styling for context lines
//! - Hunk headers (@@ ... @@) styled as section dividers
//! - File headers (--- a/ +++ b/) styled as bold path labels
//! - Line numbers in the gutter
//! - Summary line with file count and +/- totals
//! - Action hints ([a] Apply  [y] Copy  [d] Dismiss)
//!
//! Uses the TUI's semantic palette — no hardcoded colors.
//! Degrades gracefully on 16-color terminals via the palette's
//! degradation system.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use rekindle_types::patch::{PatchContent, PatchFileStatus};

/// Render a PatchContent into styled Lines for the message list.
///
/// Returns a Vec<Line> suitable for embedding in a ListItem or Paragraph.
/// The `collapsed` flag controls whether to show the full diff or just
/// the summary line (for dismissed/collapsed patches).
pub fn render_patch_lines(
    patch: &PatchContent,
    collapsed: bool,
    added_bg: Style,
    removed_bg: Style,
    context_style: Style,
    header_style: Style,
    hunk_style: Style,
    action_style: Style,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    // Description (if present)
    if let Some(ref desc) = patch.description {
        lines.push(Line::from(Span::styled(
            format!("  {desc}"),
            Style::new().add_modifier(Modifier::ITALIC),
        )));
    }

    // Summary line — always shown
    let summary = patch.summary();

    lines.push(Line::from(vec![
        Span::styled("  📋 patch: ", Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(summary, context_style),
    ]));

    if collapsed {
        lines.push(Line::from(Span::styled(
            "  (collapsed — press Enter to expand)",
            context_style,
        )));
        return lines;
    }

    // File list
    for f in &patch.files {
        let status_icon = match f.status {
            PatchFileStatus::Added => Span::styled(" + ", added_bg),
            PatchFileStatus::Modified => Span::styled(" ~ ", context_style),
            PatchFileStatus::Deleted => Span::styled(" - ", removed_bg),
            PatchFileStatus::Renamed => Span::styled(" → ", context_style),
        };
        let path_span = Span::styled(f.path.clone(), header_style);
        let stats = Span::styled(
            format!("  (+{}/-{})", f.additions, f.deletions),
            context_style,
        );
        let mut spans = vec![Span::raw("  "), status_icon, Span::raw(" "), path_span, stats];
        if let Some(ref old) = f.old_path {
            spans.push(Span::styled(format!("  (was {old})"), context_style));
        }
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(""));

    // Diff body — line by line
    for raw_line in patch.diff.lines() {
        let styled = if raw_line.starts_with('+') && !raw_line.starts_with("+++") {
            Line::from(Span::styled(format!("  {raw_line}"), added_bg))
        } else if raw_line.starts_with('-') && !raw_line.starts_with("---") {
            Line::from(Span::styled(format!("  {raw_line}"), removed_bg))
        } else if raw_line.starts_with("@@") {
            Line::from(Span::styled(format!("  {raw_line}"), hunk_style))
        } else if raw_line.starts_with("diff ") || raw_line.starts_with("--- ") || raw_line.starts_with("+++ ") {
            Line::from(Span::styled(format!("  {raw_line}"), header_style))
        } else {
            Line::from(Span::styled(format!("  {raw_line}"), context_style))
        };
        lines.push(styled);
    }

    lines.push(Line::from(""));

    // Base ref info (for conflict awareness)
    if let Some(ref base) = patch.base_ref {
        let short_ref = if base.len() > 8 { &base[..8] } else { base };
        lines.push(Line::from(Span::styled(
            format!("  base: {short_ref}"),
            context_style,
        )));
    }
    if let Some(ref branch) = patch.source_branch {
        lines.push(Line::from(Span::styled(
            format!("  branch: {branch}"),
            context_style,
        )));
    }

    // Action hints
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("[a]", action_style), Span::raw(" Apply  "),
        Span::styled("[y]", action_style), Span::raw(" Copy patch  "),
        Span::styled("[d]", action_style), Span::raw(" Dismiss"),
    ]));

    lines
}

/// Parse a ` ```patch ` fenced code block from a message body.
///
/// Returns Some(diff_text) if the message contains a patch fence,
/// None otherwise. The fence markers are stripped.
pub fn extract_patch_fence(body: &str) -> Option<String> {
    // Match "```patch" followed by end-of-line (newline or end-of-string).
    // This prevents false positives on "```patchwork" or "```patches".
    let fence_start = body.find("```patch")?;
    let after_tag = fence_start + "```patch".len();
    // The character immediately after "```patch" must be \n, \r, or end-of-string
    let next_char = body[after_tag..].chars().next();
    if next_char.is_some_and(|c| c != '\n' && c != '\r') {
        return None;
    }
    let content_start = body[fence_start..].find('\n')? + fence_start + 1;
    let fence_end = body[content_start..].find("```")?;
    let diff_text = &body[content_start..content_start + fence_end];
    if diff_text.trim().is_empty() {
        return None;
    }
    Some(diff_text.to_string())
}

/// Parse unified diff text into a PatchContent with metadata extracted
/// from the diff headers. This is the inverse of generate — it takes
/// raw diff text (e.g., from a pasted or received message) and produces
/// the structured PatchContent for rendering and application.
pub fn parse_diff_to_patch(diff_text: &str) -> PatchContent {
    let mut files = Vec::new();
    let mut current_path: Option<String> = None;
    let mut additions: u32 = 0;
    let mut deletions: u32 = 0;

    for line in diff_text.lines() {
        if let Some(path) = line.strip_prefix("+++ b/") {
            // Flush previous file
            if let Some(ref prev_path) = current_path {
                files.push(rekindle_types::patch::PatchFileMeta {
                    path: prev_path.clone(),
                    status: if additions > 0 && deletions > 0 {
                        PatchFileStatus::Modified
                    } else if deletions == 0 {
                        PatchFileStatus::Added
                    } else {
                        PatchFileStatus::Deleted
                    },
                    additions,
                    deletions,
                    old_path: None,
                });
            }
            current_path = Some(path.to_string());
            additions = 0;
            deletions = 0;
        } else if line.starts_with('+') && !line.starts_with("+++") {
            additions += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
        }
    }

    // Flush last file
    if let Some(ref prev_path) = current_path {
        files.push(rekindle_types::patch::PatchFileMeta {
            path: prev_path.clone(),
            status: if additions > 0 && deletions > 0 {
                PatchFileStatus::Modified
            } else if deletions == 0 {
                PatchFileStatus::Added
            } else {
                PatchFileStatus::Deleted
            },
            additions,
            deletions,
            old_path: None,
        });
    }

    // Handle case where no +++ headers found (malformed diff)
    if files.is_empty() && !diff_text.trim().is_empty() {
        // Count raw +/- lines as a single "unknown" file
        let total_add = diff_text.lines().filter(|l| l.starts_with('+') && !l.starts_with("+++")).count();
        let total_del = diff_text.lines().filter(|l| l.starts_with('-') && !l.starts_with("---")).count();
        if total_add > 0 || total_del > 0 {
            files.push(rekindle_types::patch::PatchFileMeta {
                path: "(unknown)".into(),
                status: PatchFileStatus::Modified,
                additions: u32::try_from(total_add).unwrap_or(u32::MAX),
                deletions: u32::try_from(total_del).unwrap_or(u32::MAX),
                old_path: None,
            });
        }
    }

    PatchContent {
        diff: diff_text.to_string(),
        files,
        description: None,
        base_ref: None,
        source_branch: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_patch_fence_basic() {
        let body = "Here's the fix:\n\n```patch\n--- a/foo.rs\n+++ b/foo.rs\n@@ -1 +1 @@\n-old\n+new\n```\n\nLet me know.";
        let patch = extract_patch_fence(body).unwrap();
        assert!(patch.contains("-old"));
        assert!(patch.contains("+new"));
    }

    #[test]
    fn extract_patch_fence_empty() {
        let body = "No patch here.";
        assert!(extract_patch_fence(body).is_none());
    }

    #[test]
    fn extract_patch_fence_empty_fence() {
        let body = "```patch\n```";
        assert!(extract_patch_fence(body).is_none());
    }

    #[test]
    fn parse_diff_single_file() {
        let diff = "diff --git a/foo.rs b/foo.rs\n--- a/foo.rs\n+++ b/foo.rs\n@@ -1 +1 @@\n-old line\n+new line\n";
        let patch = parse_diff_to_patch(diff);
        assert_eq!(patch.files.len(), 1);
        assert_eq!(patch.files[0].path, "foo.rs");
        assert_eq!(patch.files[0].additions, 1);
        assert_eq!(patch.files[0].deletions, 1);
        assert_eq!(patch.files[0].status, PatchFileStatus::Modified);
    }

    #[test]
    fn parse_diff_multi_file() {
        let diff = "--- a/a.rs\n+++ b/a.rs\n+added\n--- a/b.rs\n+++ b/b.rs\n-removed\n";
        let patch = parse_diff_to_patch(diff);
        assert_eq!(patch.files.len(), 2);
        assert_eq!(patch.files[0].path, "a.rs");
        assert_eq!(patch.files[0].status, PatchFileStatus::Added);
        assert_eq!(patch.files[1].path, "b.rs");
        assert_eq!(patch.files[1].status, PatchFileStatus::Deleted);
    }

    #[test]
    fn parse_diff_malformed_still_counts() {
        let diff = "+added line\n-removed line\n";
        let patch = parse_diff_to_patch(diff);
        assert_eq!(patch.files.len(), 1);
        assert_eq!(patch.files[0].path, "(unknown)");
        assert_eq!(patch.files[0].additions, 1);
        assert_eq!(patch.files[0].deletions, 1);
    }

    #[test]
    fn render_collapsed_shows_summary_only() {
        let patch = PatchContent {
            diff: "--- a/foo.rs\n+++ b/foo.rs\n-old\n+new\n".into(),
            files: vec![rekindle_types::patch::PatchFileMeta {
                path: "foo.rs".into(),
                status: PatchFileStatus::Modified,
                additions: 1, deletions: 1, old_path: None,
            }],
            description: None, base_ref: None, source_branch: None,
        };
        let lines = render_patch_lines(
            &patch, true,
            Style::new(), Style::new(), Style::new(),
            Style::new(), Style::new(), Style::new(),
        );
        // Collapsed should have summary + collapsed hint, no diff lines
        assert!(lines.len() <= 4);
        let text: String = lines.iter().map(|l| l.to_string()).collect();
        assert!(text.contains("collapsed"));
    }

    #[test]
    fn render_expanded_shows_diff_lines() {
        let patch = PatchContent {
            diff: "--- a/foo.rs\n+++ b/foo.rs\n@@ -1 +1 @@\n-old\n+new\n".into(),
            files: vec![rekindle_types::patch::PatchFileMeta {
                path: "foo.rs".into(),
                status: PatchFileStatus::Modified,
                additions: 1, deletions: 1, old_path: None,
            }],
            description: Some("Fix the thing".into()),
            base_ref: Some("abc12345".into()),
            source_branch: Some("fix/thing".into()),
        };
        let lines = render_patch_lines(
            &patch, false,
            Style::new(), Style::new(), Style::new(),
            Style::new(), Style::new(), Style::new(),
        );
        let text: String = lines.iter().map(|l| l.to_string()).collect();
        assert!(text.contains("-old"));
        assert!(text.contains("+new"));
        assert!(text.contains("Fix the thing"));
        assert!(text.contains("abc12345"));
        assert!(text.contains("fix/thing"));
        assert!(text.contains("[a]"));
        assert!(text.contains("Apply"));
    }
}
