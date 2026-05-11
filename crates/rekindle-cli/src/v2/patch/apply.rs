//! Patch application — apply a received PatchContent to the local working tree.
//!
//! Security: All diff paths are validated before application. Paths containing
//! `..`, absolute paths, and paths resolving outside the repository root are
//! rejected. This prevents a malicious peer from crafting a diff that writes
//! to arbitrary filesystem locations (e.g., .git/hooks/, ~/.ssh/).

use std::path::Path;

use rekindle_types::patch::PatchContent;

/// Result of applying a patch.
#[derive(Debug)]
pub struct ApplyResult {
    /// Files successfully modified.
    pub applied_files: Vec<String>,
    /// Whether the base ref matched the local HEAD (clean apply).
    pub clean_apply: bool,
    /// If base ref didn't match, what the local HEAD was.
    pub local_head: Option<String>,
    /// Error message if application failed.
    pub error: Option<String>,
}

/// Validate that all file paths in a parsed diff are safe to apply.
///
/// Rejects:
/// - Absolute paths (starting with `/`)
/// - Path traversal (`..` component anywhere)
/// - Paths resolving outside the repository working directory
/// - Paths targeting `.git/` internals
///
/// Returns Ok(()) if all paths are safe, Err(description) if any are dangerous.
fn validate_diff_paths(diff: &git2::Diff<'_>, repo_workdir: &Path) -> Result<(), String> {
    let num_deltas = diff.deltas().len();
    for i in 0..num_deltas {
        let delta = diff.get_delta(i).expect("delta index valid");

        for file in [delta.old_file(), delta.new_file()] {
            let Some(path) = file.path() else { continue };
            let path_str = path.to_string_lossy();

            // Reject absolute paths
            if path.is_absolute() {
                return Err(format!("absolute path rejected: {path_str}"));
            }

            // Reject path traversal components
            for component in path.components() {
                if matches!(component, std::path::Component::ParentDir) {
                    return Err(format!("path traversal rejected: {path_str}"));
                }
            }

            // Reject .git/ internal paths
            let first_component = path.components().next();
            if first_component.is_some_and(|c| c.as_os_str() == ".git") {
                return Err(format!("git internal path rejected: {path_str}"));
            }

            // Defense-in-depth: verify the resolved path stays within the repo.
            // NOTE: For new files in new directories, parent.canonicalize() will
            // fail (directory doesn't exist yet) and this check silently passes.
            // The Component::ParentDir check above is the primary security boundary
            // that catches path traversal. This canonicalize check adds protection
            // against symlink-based escapes where the symlink already exists in the
            // repo — a strictly harder attack vector.
            let resolved = repo_workdir.join(path);
            let parent = resolved.parent().unwrap_or(repo_workdir);
            if let Ok(canonical_parent) = parent.canonicalize() {
                if let Ok(canonical_workdir) = repo_workdir.canonicalize() {
                    if !canonical_parent.starts_with(&canonical_workdir) {
                        return Err(format!(
                            "path escapes repository root: {path_str} resolves to {}",
                            canonical_parent.display()
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Apply a PatchContent to the local working tree.
///
/// Returns `ApplyResult` with details about what was applied.
/// Does NOT commit — changes are left in the working tree for review.
pub fn apply_patch(repo_path: &Path, patch: &PatchContent) -> ApplyResult {
    let repo = match git2::Repository::discover(repo_path) {
        Ok(r) => r,
        Err(e) => return ApplyResult {
            applied_files: Vec::new(),
            clean_apply: false,
            local_head: None,
            error: Some(format!("not a git repository: {e}")),
        },
    };

    let repo_workdir = match repo.workdir() {
        Some(w) => w.to_path_buf(),
        None => return ApplyResult {
            applied_files: Vec::new(),
            clean_apply: false,
            local_head: None,
            error: Some("bare repository — cannot apply patches".into()),
        },
    };

    // Conflict detection: compare base_ref against local HEAD
    let local_head = repo.head()
        .ok()
        .and_then(|r| r.target())
        .map(|oid| oid.to_string());

    let clean_apply = match (&patch.base_ref, &local_head) {
        (Some(base), Some(head)) => base == head,
        (None, _) => true,
        (Some(_), None) => false,
    };

    // Parse the diff text into a git2::Diff
    let diff = match git2::Diff::from_buffer(patch.diff.as_bytes()) {
        Ok(d) => d,
        Err(e) => return ApplyResult {
            applied_files: Vec::new(),
            clean_apply,
            local_head,
            error: Some(format!("failed to parse patch: {e}")),
        },
    };

    // SECURITY: Validate all file paths before applying
    if let Err(path_error) = validate_diff_paths(&diff, &repo_workdir) {
        return ApplyResult {
            applied_files: Vec::new(),
            clean_apply,
            local_head,
            error: Some(format!("SECURITY: patch rejected — {path_error}")),
        };
    }

    // Apply the diff to the working directory
    match repo.apply(&diff, git2::ApplyLocation::WorkDir, None) {
        Ok(()) => {
            let applied_files = patch.files.iter()
                .map(|f| f.path.clone())
                .collect();

            ApplyResult {
                applied_files,
                clean_apply,
                local_head,
                error: None,
            }
        }
        Err(e) => ApplyResult {
            applied_files: Vec::new(),
            clean_apply,
            local_head,
            error: Some(format!("patch application failed: {e}")),
        },
    }
}

/// Check if a patch can be applied cleanly without actually applying it.
///
/// Returns None if clean, Some(error) if conflicts or security issues.
pub fn check_patch(repo_path: &Path, patch: &PatchContent) -> Option<String> {
    let repo = match git2::Repository::discover(repo_path) {
        Ok(r) => r,
        Err(e) => return Some(format!("not a git repository: {e}")),
    };

    let repo_workdir = match repo.workdir() {
        Some(w) => w.to_path_buf(),
        None => return Some("bare repository — cannot apply patches".into()),
    };

    let diff = match git2::Diff::from_buffer(patch.diff.as_bytes()) {
        Ok(d) => d,
        Err(e) => return Some(format!("invalid patch: {e}")),
    };

    // SECURITY: Validate paths even for dry-run checks
    if let Err(path_error) = validate_diff_paths(&diff, &repo_workdir) {
        return Some(format!("SECURITY: patch rejected — {path_error}"));
    }

    // apply_to_tree returns a new Index without modifying the repo's
    // working directory or actual index — a true dry-run check.
    let head_tree = match repo.head().and_then(|r| r.peel_to_tree()) {
        Ok(t) => t,
        Err(e) => return Some(format!("cannot read HEAD tree: {e}")),
    };
    match repo.apply_to_tree(&head_tree, &diff, None) {
        Ok(_new_index) => None,
        Err(e) => Some(format!("patch would conflict: {e}")),
    }
}
