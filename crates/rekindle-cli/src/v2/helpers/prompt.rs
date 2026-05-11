//! TTY-aware interactive prompts with zeroize support for secrets.

use std::io::IsTerminal;
use anyhow::Context;

use super::validate::validate_display_name;

/// Prompt for typed confirmation of a destructive operation.
/// Non-interactive stdin returns an error.
pub fn confirm_destructive(prompt: &str, phrase: &str) -> anyhow::Result<bool> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "destructive operation requires interactive confirmation\n\
             pass --yes to skip (if supported) or run in a terminal"
        );
    }

    let input: String = dialoguer::Input::new()
        .with_prompt(format!("{prompt}\nType \"{phrase}\" to confirm"))
        .interact_text()
        .context("failed to read confirmation")?;

    Ok(input.trim() == phrase)
}

/// Prompt for yes/no confirmation. Default is false (safe default).
pub fn confirm(prompt: &str) -> anyhow::Result<bool> {
    if !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "confirmation required but stdin is not a terminal\n\
             pass --yes to skip or run in a terminal"
        );
    }

    dialoguer::Confirm::new()
        .with_prompt(prompt)
        .default(false)
        .interact()
        .context("failed to read confirmation")
}

/// Prompt for a password with zeroize-on-drop.
/// Piped stdin: reads raw, trims trailing newline.
pub fn prompt_password(prompt: &str) -> anyhow::Result<zeroize::Zeroizing<String>> {
    if !std::io::stdin().is_terminal() {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
            .context("failed to read password from stdin")?;
        if buf.ends_with('\n') { buf.pop(); }
        if buf.ends_with('\r') { buf.pop(); }
        if buf.is_empty() {
            anyhow::bail!("empty password from stdin — refusing to proceed");
        }
        return Ok(zeroize::Zeroizing::new(buf));
    }

    let pass = dialoguer::Password::new()
        .with_prompt(prompt)
        .interact()
        .context("failed to read password")?;

    if pass.is_empty() {
        anyhow::bail!("empty password — refusing to proceed");
    }

    Ok(zeroize::Zeroizing::new(pass))
}

/// Resolve a display name: use provided value, prompt interactively, or error.
pub fn resolve_display_name(provided: Option<&str>) -> anyhow::Result<String> {
    if let Some(name) = provided {
        return validate_display_name(name);
    }

    if !std::io::stdin().is_terminal() {
        anyhow::bail!(
            "display name required in non-interactive mode\n\
             pass --display-name <NAME>"
        );
    }

    let name: String = dialoguer::Input::new()
        .with_prompt("Display name")
        .interact_text()
        .context("failed to read display name")?;

    validate_display_name(&name)
}
