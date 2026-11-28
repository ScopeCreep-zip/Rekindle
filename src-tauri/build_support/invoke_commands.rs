//! Parser for the command entries in `src/invoke.rs`.
//!
//! Shared by `build.rs` (which feeds the names into
//! `AppManifest::commands`, so `tauri-build` autogenerates one
//! `allow-$cmd`/`deny-$cmd` permission per command) and by
//! `tests/capability_policy.rs` (which checks every command is granted to
//! exactly the windows that may call it). `invoke.rs` is the single source
//! of truth — there is no second hand-maintained list to drift.
//!
//! `invoke.rs` keeps one `*_commands!` macro per domain, folded into the
//! single `generate_handler!` list. Each domain's entries sit between a line
//! ending in [`LIST_OPEN`] and the line [`LIST_CLOSE`]. The grammar is
//! deliberately narrow: between them every non-blank line is a `//` comment,
//! a `#[cfg(debug_assertions)]` gate, or a `path::to::command,` entry.
//! Anything else is rejected so a new construct can't silently drop a
//! command from the ACL.

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

/// The end of the line that opens a domain macro's entries.
pub const LIST_OPEN: &str = "$next!([$($done)*";

/// The line that closes a domain macro's entries.
pub const LIST_CLOSE: &str = "] $($rest)*) };";

/// Parse every domain list's entries in `src`.
pub fn parse(src: &str) -> Result<Vec<InvokeCommand>, String> {
    let mut out: Vec<InvokeCommand> = Vec::new();
    let mut lines = src.lines().enumerate();
    while let Some((_, raw)) = lines.next() {
        if raw.trim_end().ends_with(LIST_OPEN) {
            parse_list(&mut lines, &mut out)?;
        }
    }
    if out.is_empty() {
        return Err("invoke.rs registers no command".to_owned());
    }
    Ok(out)
}

/// Parse one domain list up to its [`LIST_CLOSE`] line into `out`.
fn parse_list<'a>(
    lines: &mut impl Iterator<Item = (usize, &'a str)>,
    out: &mut Vec<InvokeCommand>,
) -> Result<(), String> {
    let mut gated = false;
    for (idx, raw) in lines {
        let line = raw.split("//").next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if line == LIST_CLOSE {
            if gated {
                return Err(format!("invoke.rs line {idx}: dangling debug gate"));
            }
            return Ok(());
        }
        if line == DEBUG_GATE {
            if gated {
                return Err(format!("invoke.rs line {idx}: duplicate debug gate"));
            }
            gated = true;
            continue;
        }
        let name = entry_name(line).ok_or_else(|| {
            format!("invoke.rs line {idx}: expected `path::command,`, got `{line}`")
        })?;
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
    Err("invoke.rs: a domain list has no closing line".to_owned())
}

/// The bare command name of a `path::to::command,` entry.
fn entry_name(line: &str) -> Option<String> {
    let path = line.strip_suffix(',')?;
    let valid = path.split("::").all(|seg| {
        !seg.is_empty()
            && seg
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    });
    valid.then(|| path.rsplit("::").next().unwrap_or(path).to_owned())
}
