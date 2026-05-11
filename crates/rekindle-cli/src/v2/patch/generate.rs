//! Patch generation — create PatchContent from local git working tree changes.
//!
//! Uses git2 (libgit2) to diff HEAD against the working tree or index.
//! No git CLI dependency. Works on any platform with libgit2.

use std::path::Path;

use rekindle_types::patch::{PatchContent, PatchFileMeta, PatchFileStatus};

/// Generate a PatchContent from changes in the working tree relative to HEAD.
///
/// If `paths` is non-empty, only diffs for those paths are included.
/// If `paths` is empty, all changes in the working tree are included.
/// If `staged_only` is true, only staged (index) changes are included.
pub fn generate_patch(
    repo_path: &Path,
    paths: &[&str],
    staged_only: bool,
) -> anyhow::Result<PatchContent> {
    let repo = git2::Repository::discover(repo_path)
        .map_err(|e| anyhow::anyhow!("not a git repository: {e}"))?;

    let head = repo.head()
        .and_then(|r| r.peel_to_tree())
        .map_err(|e| anyhow::anyhow!("cannot read HEAD tree: {e}"))?;

    let mut diff_opts = git2::DiffOptions::new();
    diff_opts.context_lines(3);
    diff_opts.ignore_whitespace_eol(true);

    for path in paths {
        diff_opts.pathspec(path);
    }

    let diff = if staged_only {
        repo.diff_tree_to_index(Some(&head), None, Some(&mut diff_opts))
    } else {
        repo.diff_tree_to_workdir_with_index(Some(&head), Some(&mut diff_opts))
    }.map_err(|e| anyhow::anyhow!("diff failed: {e}"))?;

    // Detect renames
    let mut find_opts = git2::DiffFindOptions::new();
    find_opts.renames(true);
    find_opts.copies(false);
    let mut diff = diff;
    diff.find_similar(Some(&mut find_opts))
        .map_err(|e| anyhow::anyhow!("rename detection failed: {e}"))?;

    // Extract per-file metadata
    let stats = diff.stats().map_err(|e| anyhow::anyhow!("diff stats failed: {e}"))?;
    let _ = stats; // stats are aggregate — we compute per-file below

    let mut files = Vec::new();
    let num_deltas = diff.deltas().len();
    for i in 0..num_deltas {
        let delta = diff.get_delta(i).expect("delta index valid");
        let status = match delta.status() {
            git2::Delta::Added | git2::Delta::Untracked | git2::Delta::Copied => PatchFileStatus::Added,
            git2::Delta::Deleted => PatchFileStatus::Deleted,
            git2::Delta::Renamed => PatchFileStatus::Renamed,
            _ => PatchFileStatus::Modified,
        };

        let new_path = delta.new_file().path()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let old_path = if status == PatchFileStatus::Renamed {
            delta.old_file().path().map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };

        // Count additions/deletions for this file by examining hunks
        let mut additions = 0u32;
        let mut deletions = 0u32;
        if let Ok(Some(ref patch)) = git2::Patch::from_diff(&diff, i) {
            let (_, adds, dels) = patch.line_stats().unwrap_or((0, 0, 0));
            additions = u32::try_from(adds).unwrap_or(u32::MAX);
            deletions = u32::try_from(dels).unwrap_or(u32::MAX);
        }

        files.push(PatchFileMeta {
            path: new_path,
            status,
            additions,
            deletions,
            old_path,
        });
    }

    // Generate the unified diff text
    let mut diff_text = String::new();
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        let origin = line.origin();
        match origin {
            '+' | '-' | ' ' => {
                diff_text.push(origin);
                diff_text.push_str(std::str::from_utf8(line.content()).unwrap_or(""));
            }
            'F' | 'H' => {
                // File header or hunk header — include as-is
                diff_text.push_str(std::str::from_utf8(line.content()).unwrap_or(""));
            }
            _ => {}
        }
        true
    }).map_err(|e| anyhow::anyhow!("diff print failed: {e}"))?;

    // Get the current HEAD ref for conflict detection
    let base_ref = repo.head()
        .and_then(|r| r.target().ok_or_else(|| git2::Error::from_str("no target")))
        .map(|oid| oid.to_string())
        .ok();

    let source_branch = repo.head()
        .ok()
        .and_then(|r| r.shorthand().map(str::to_string));

    Ok(PatchContent {
        diff: diff_text,
        files,
        description: None,
        base_ref,
        source_branch,
    })
}

/// Generate a patch of all staged changes.
pub fn generate_staged_patch() -> anyhow::Result<PatchContent> {
    let cwd = std::env::current_dir()?;
    generate_patch(&cwd, &[], true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_from_non_repo_fails() {
        let result = generate_patch(Path::new("/tmp"), &[], false);
        assert!(result.is_err());
    }
}
