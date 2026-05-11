//! Patch system — generate, render, and apply code change patches.
//!
//! Three capabilities:
//! - `generate` — create a PatchContent from local git working tree changes
//! - `render` — produce styled ratatui widgets from unified diff text
//! - `apply` — apply a received PatchContent to the local working tree
//!
//! The generate and apply modules use `git2` (libgit2) — no git CLI dependency.
//! The render module uses ratatui spans with the TUI theme palette — no syntect
//! dependency yet (syntax highlighting is a future enhancement over the
//! already-functional diff coloring).

pub mod apply;
pub mod generate;
pub mod render;
