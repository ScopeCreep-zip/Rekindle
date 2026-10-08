//! Parser for the `tauri::generate_handler![…]` list in `src/invoke.rs`.
//!
//! Shared by `build.rs` (which feeds the names into
//! `AppManifest::commands`, so `tauri-build` autogenerates one
//! `allow-$cmd`/`deny-$cmd` permission per command) and by
//! `tests/capability_policy.rs` (which checks every command is granted to
//! exactly the windows that may call it). `invoke.rs` is the single source
//! of truth — there is no second hand-maintained list to drift.
//!
//! The grammar is deliberately narrow: inside the macro body every
//! non-blank line is a `//` comment, a `#[cfg(debug_assertions)]` gate, or
//! a `path::to::command,` entry. Anything else is rejected so a new
//! construct can't silently drop a command from the ACL.

/// One `generate_handler!` entry, by bare function name — what
/// `generate_handler!` dispatches on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvokeCommand {
    /// Registered in every build.
    Release(String),
    /// Gated by `#[cfg(debug_assertions)]`: compiled out of release.
    DebugOnly(String),
}

impl InvokeCommand {
    pub fn name(&self) -> &str {
        match self {
            Self::Release(n) | Self::DebugOnly(n) => n,
        }
    }
}

const DEBUG_GATE: &str = "#[cfg(debug_assertions)]";

/// Parse every entry of the `generate_handler![…]` list in `src`.
pub fn parse(src: &str) -> Result<Vec<InvokeCommand>, String> {
    let start = src
        .find("generate_handler![")
        .ok_or("no `generate_handler![` in invoke.rs")?;
    let body = &src[start + "generate_handler![".len()..];

    let mut out: Vec<InvokeCommand> = Vec::new();
    let mut gated = false;
    let mut closed = false;
    for (idx, raw) in body.lines().enumerate() {
        let line = raw.split("//").next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if line == "]" {
            closed = true;
            break;
        }
        if line == DEBUG_GATE {
            if gated {
                return Err(format!("invoke.rs list line {idx}: duplicate debug gate"));
            }
            gated = true;
            continue;
        }
        let path = line.strip_suffix(',').ok_or_else(|| {
            format!("invoke.rs list line {idx}: expected `path::command,`, got `{line}`")
        })?;
        let valid = path.split("::").all(|seg| {
            !seg.is_empty()
                && seg
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        });
        if !valid {
            return Err(format!(
                "invoke.rs list line {idx}: unsupported entry `{line}`"
            ));
        }
        let name = path.rsplit("::").next().unwrap_or(path).to_owned();
        if out.iter().any(|c| c.name() == name) {
            return Err(format!("invoke.rs registers `{name}` twice"));
        }
        out.push(if gated {
            InvokeCommand::DebugOnly(name)
        } else {
            InvokeCommand::Release(name)
        });
        gated = false;
    }
    if !closed {
        return Err("unterminated `generate_handler![` in invoke.rs".to_owned());
    }
    if gated {
        return Err("invoke.rs list ends with a dangling debug gate".to_owned());
    }
    if out.is_empty() {
        return Err("invoke.rs `generate_handler!` list is empty".to_owned());
    }
    Ok(out)
}
