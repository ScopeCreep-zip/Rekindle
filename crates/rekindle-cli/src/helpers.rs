//! Shared utility functions used across CLI command modules.
//!
//! Every function here is a boundary-layer primitive: input validation,
//! output formatting, TTY detection, tracing initialization, and secret
//! handling. No business logic — that lives in command modules.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::Context;

/// The identity registry, in the data root's `state` directory
/// (`rekindle_utils::paths::DataRoot`). Resolved only; the daemon writes it.
pub fn registry_path() -> anyhow::Result<PathBuf> {
    Ok(rekindle_utils::paths::DataRoot::resolve()?.registry())
}

/// The daemon's Veilid storage directory in the data root, or
/// `override_path`. Resolved only; the daemon creates it.
pub fn storage_dir(override_path: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(p) = override_path {
        return Ok(p.to_path_buf());
    }
    Ok(rekindle_utils::paths::DataRoot::resolve()?.veilid())
}

// ── Input Sanitization ──────────────────────────────────────────────────

/// Validate a display name.
///
/// Rules:
/// - 1-64 characters after trimming whitespace
/// - No control characters
/// - No leading/trailing whitespace (trimmed automatically)
pub fn validate_display_name(name: &str) -> anyhow::Result<String> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("display name cannot be empty");
    }
    if trimmed.len() > 64 {
        anyhow::bail!("display name too long ({} chars, max 64)", trimmed.len());
    }
    if trimmed.chars().any(char::is_control) {
        anyhow::bail!("display name cannot contain control characters");
    }
    Ok(trimmed)
}

/// Validate a community or channel name.
///
/// Rules:
/// - 1-100 characters
/// - No control characters
/// - Alphanumeric, hyphens, underscores, spaces allowed
pub fn validate_name(name: &str, label: &str) -> anyhow::Result<String> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("{label} name cannot be empty");
    }
    if trimmed.len() > 100 {
        anyhow::bail!("{label} name too long ({} chars, max 100)", trimmed.len());
    }
    if trimmed.chars().any(char::is_control) {
        anyhow::bail!("{label} name cannot contain control characters");
    }
    Ok(trimmed)
}

// ── TTY-Aware Prompts ───────────────────────────────────────────────────
//
// clig.dev: prompt only when stdin is a terminal, never when `--no-input`
// is passed; otherwise fail and name the flag that supplies the input.

/// Global `--no-input` flag, set once at startup.
static NO_INPUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Disable every prompt. Called once from `main.rs` for `--no-input`.
pub fn set_no_input(no_input: bool) {
    NO_INPUT.store(no_input, std::sync::atomic::Ordering::Relaxed);
}

/// Whether a prompt may be shown: stdin is a terminal and `--no-input`
/// was not passed.
fn interactive() -> bool {
    !NO_INPUT.load(std::sync::atomic::Ordering::Relaxed) && std::io::stdin().is_terminal()
}

/// A yes/no prompt, default No. Without a terminal it is an error naming
/// `flag`, the non-interactive way to answer.
fn ask_yes_no(prompt: &str, flag: &str) -> anyhow::Result<bool> {
    if !interactive() {
        anyhow::bail!("confirmation needed and there is no terminal to ask on: pass {flag}");
    }
    dialoguer::Confirm::new()
        .with_prompt(prompt)
        .default(false)
        .interact()
        .context("failed to read confirmation")
}

/// A typed answer. Without a terminal it is an error naming `flag`.
fn ask_text(prompt: &str, flag: &str) -> anyhow::Result<String> {
    if !interactive() {
        anyhow::bail!("confirmation needed and there is no terminal to ask on: pass {flag}");
    }
    dialoguer::Input::new()
        .with_prompt(prompt)
        .interact_text()
        .context("failed to read confirmation")
}

/// Read a passphrase (clig.dev: never from a flag value or the
/// environment). From `file` when given (`-` is stdin), otherwise from a
/// terminal prompt with echo off. Refuses an empty passphrase.
pub fn read_passphrase(
    prompt: &str,
    file: Option<&Path>,
) -> anyhow::Result<zeroize::Zeroizing<String>> {
    let pass = match file {
        Some(path) => {
            let mut buf = zeroize::Zeroizing::new(String::new());
            if path == Path::new("-") {
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
                    .context("failed to read the passphrase from stdin")?;
            } else {
                let mut file = std::fs::File::open(path)
                    .with_context(|| format!("cannot open {}", path.display()))?;
                std::io::Read::read_to_string(&mut file, &mut buf)
                    .with_context(|| format!("cannot read {}", path.display()))?;
            }
            // One trailing line ending is the file's, not the passphrase's.
            let trimmed = buf
                .strip_suffix("\r\n")
                .or_else(|| buf.strip_suffix('\n'))
                .unwrap_or(&buf);
            zeroize::Zeroizing::new(trimmed.to_owned())
        }
        None if interactive() => zeroize::Zeroizing::new(
            dialoguer::Password::new()
                .with_prompt(prompt)
                .interact()
                .context("failed to read the passphrase")?,
        ),
        None => anyhow::bail!(
            "a passphrase is needed and there is no terminal to ask on: \
             pass --passphrase-file <PATH> (or - for stdin)"
        ),
    };
    if pass.is_empty() {
        anyhow::bail!("empty passphrase — refusing to proceed");
    }
    Ok(pass)
}

/// Prompt for a display name interactively, or use the provided value.
///
/// If `provided` is `Some`, validates and returns it.
/// If `None` and stdin is a TTY, prompts the user.
/// If `None` and stdin is piped, returns an error.
pub fn resolve_display_name(provided: Option<&str>) -> anyhow::Result<String> {
    if let Some(name) = provided {
        return validate_display_name(name);
    }

    if !interactive() {
        anyhow::bail!("a display name is needed and there is no terminal to ask on: pass --display-name <NAME>");
    }

    let name: String = dialoguer::Input::new()
        .with_prompt("Display name")
        .interact_text()
        .context("failed to read display name")?;

    validate_display_name(&name)
}

#[path = "helpers/prove.rs"]
mod prove;

pub use prove::{offer_passphrase, send, set_passphrase_file, set_proof_flags};

#[cfg(test)]
#[path = "helpers/tests.rs"]
mod tests;
