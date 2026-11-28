//! Keybinding system — compile-time-embedded JSON keymap.
//!
//! The keymap is the single source of truth for all TUI keybindings.
//! It is loaded from `keymap/default.keymap.json` via `include_str!` at
//! compile time. The same data drives:
//! 1. Runtime key dispatch (`classify`)
//! 2. Help overlay text (`help_text`)
//! 3. Status bar hints (`hint_text`)
//!
//! Source patterns:
//! - schemaui `src/tui/app/keymap.rs` — `include_str!` + serde_json + context scoping
//! - siggy `src/keybindings.rs` — three-mode HashMap lookup

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::action::Action;

/// The embedded keymap JSON — compiled into the binary.
const KEYMAP_JSON: &str = include_str!("../keymap/default.keymap.json");

/// Context in which a keybinding is active.
///
/// A binding only fires when the app's current context matches one of
/// the binding's declared contexts. This prevents navigation keys from
/// interfering with text input, search queries, or overlay interactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub enum KeymapContext {
    /// Normal navigation mode — the default.
    Default,
    /// Text input mode — keys go to the input box.
    Input,
    /// Search overlay is active.
    Search,
    /// A modal overlay is active (help, confirm, etc.).
    Overlay,
}

/// A parsed key combination: modifier flags + key code + display string.
#[derive(Debug, Clone)]
struct KeyPattern {
    code: KeyCode,
    modifiers: KeyModifiers,
    display: String,
}

/// A single keybinding entry from the JSON keymap.
#[derive(Debug, Clone)]
struct KeyBinding {
    /// Unique identifier — read by conflict detection test
    /// and diagnostic logging.
    id: String,
    description: String,
    contexts: Vec<KeymapContext>,
    action: String,
    combos: Vec<KeyPattern>,
    /// Whether this binding fires an Action when matched.
    /// When false, the binding exists only for help text display
    /// (e.g., cursor movement keys handled natively by tui-textarea,
    /// or future two-key prefix sequences like `gg`).
    dispatch: bool,
}

/// The keymap store — loaded once at startup, immutable thereafter.
///
/// Provides `classify()` for runtime dispatch and `help_text()` for
/// the help overlay and status bar.
pub struct KeymapStore {
    bindings: Vec<KeyBinding>,
}

impl KeymapStore {
    /// Parse the embedded JSON keymap.
    ///
    /// Fails if the JSON is malformed or if a combo string can't be parsed.
    /// This runs at startup — a parse error is a build-time bug, not a
    /// user-facing error.
    pub fn load() -> anyhow::Result<Self> {
        let raw: Vec<RawBinding> = serde_json::from_str(KEYMAP_JSON)
            .map_err(|e| anyhow::anyhow!("keymap parse failed: {e}"))?;

        let mut bindings = Vec::with_capacity(raw.len());
        for entry in raw {
            let combos: Vec<KeyPattern> = entry
                .combos
                .iter()
                .map(|s| parse_combo(s))
                .collect::<anyhow::Result<Vec<_>>>()?;

            bindings.push(KeyBinding {
                id: entry.id,
                description: entry.description,
                contexts: entry.contexts,
                action: entry.action,
                combos,
                dispatch: entry.dispatch.unwrap_or(true),
            });
        }

        Ok(Self { bindings })
    }

    /// Classify a key event into an Action, given the current context.
    ///
    /// Iterates all bindings, checks context membership, then checks
    /// each combo against the incoming key event. First match wins.
    ///
    /// Returns `None` if no binding matches — the key is unconsumed.
    pub fn classify(&self, key: KeyEvent, context: KeymapContext) -> Option<Action> {
        for binding in &self.bindings {
            if !binding.dispatch || !binding.contexts.contains(&context) {
                continue;
            }
            for combo in &binding.combos {
                if matches_key(combo, &key) {
                    tracing::trace!(
                        binding_id = binding.id.as_str(),
                        action = binding.action.as_str(),
                        "keybinding matched"
                    );
                    return map_action(&binding.action);
                }
            }
        }
        None
    }

    /// Generate help text for a given context.
    ///
    /// Returns `(display_combo, description)` pairs for all bindings
    /// active in the given context. Used by the help overlay and
    /// the status bar hint line.
    pub fn help_text(&self, context: KeymapContext) -> Vec<(&str, &str)> {
        self.bindings
            .iter()
            .filter(|b| b.contexts.contains(&context))
            .filter_map(|b| {
                b.combos
                    .first()
                    .map(|c| (c.display.as_str(), b.description.as_str()))
            })
            .collect()
    }

    /// Generate a compact hint string for the status bar.
    ///
    /// Returns e.g. `"q quit | ? help | Tab focus | / search"`
    pub fn hint_line(&self, context: KeymapContext) -> String {
        let hints = self.help_text(context);
        hints
            .iter()
            .take(6) // at most 6 hints in the bar
            .map(|(combo, desc)| format!("{combo} {desc}"))
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

// ── JSON deserialization ────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct RawBinding {
    id: String,
    description: String,
    contexts: Vec<KeymapContext>,
    action: String,
    combos: Vec<String>,
    /// Whether this binding fires an Action. Defaults to true if omitted.
    dispatch: Option<bool>,
}

// ── Combo string parsing ────────────────────────────────────────────────

/// Parse a combo string like "Ctrl+k", "Shift+Tab", "Enter", "?" into
/// a `KeyPattern`.
///
/// Supported modifiers: `Ctrl+`, `Shift+`, `Alt+`.
/// Supported keys: single chars, `Tab`, `Enter`, `Esc`, `Space`,
/// `Up`, `Down`, `Left`, `Right`, `Home`, `End`, `PageUp`, `PageDown`,
/// `Backspace`, `Delete`.
fn parse_combo(s: &str) -> anyhow::Result<KeyPattern> {
    let mut modifiers = KeyModifiers::empty();
    let mut remaining = s;

    // Extract modifiers
    loop {
        if let Some(rest) = remaining.strip_prefix("Ctrl+") {
            modifiers |= KeyModifiers::CONTROL;
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix("Shift+") {
            modifiers |= KeyModifiers::SHIFT;
            remaining = rest;
        } else if let Some(rest) = remaining.strip_prefix("Alt+") {
            modifiers |= KeyModifiers::ALT;
            remaining = rest;
        } else {
            break;
        }
    }

    let code = match remaining {
        "Tab" => KeyCode::Tab,
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Space" => KeyCode::Char(' '),
        "Backspace" => KeyCode::Backspace,
        "Delete" => KeyCode::Delete,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        s if s.len() == 1 => {
            let ch = s.chars().next().expect("non-empty string");
            KeyCode::Char(ch)
        }
        other => {
            anyhow::bail!("unknown key in combo '{s}': '{other}'");
        }
    };

    // Shift+Tab is actually BackTab in crossterm
    let code = if code == KeyCode::Tab && modifiers.contains(KeyModifiers::SHIFT) {
        modifiers.remove(KeyModifiers::SHIFT);
        KeyCode::BackTab
    } else {
        code
    };

    Ok(KeyPattern {
        code,
        modifiers,
        display: s.to_string(),
    })
}

/// Check if a parsed KeyPattern matches a crossterm KeyEvent.
fn matches_key(pattern: &KeyPattern, event: &KeyEvent) -> bool {
    if pattern.code != event.code {
        return false;
    }

    // For character keys, crossterm may set SHIFT when the char is uppercase.
    // We need to handle this: 'G' in the keymap means KeyCode::Char('G')
    // which crossterm reports with SHIFT modifier. So we mask out SHIFT
    // when comparing char keys if the pattern doesn't explicitly require SHIFT.
    if let KeyCode::Char(c) = pattern.code {
        if c.is_ascii_uppercase() && !pattern.modifiers.contains(KeyModifiers::SHIFT) {
            let event_mods = event.modifiers & !KeyModifiers::SHIFT;
            let pattern_mods = pattern.modifiers & !KeyModifiers::SHIFT;
            return event_mods == pattern_mods;
        }
    }

    // BackTab (Shift+Tab): crossterm sends KeyCode::BackTab with SHIFT
    // still set on some terminals. Our combo parser strips SHIFT when
    // converting "Shift+Tab" to BackTab (since BackTab already implies it).
    // Accept BackTab with or without the SHIFT modifier in the event.
    if pattern.code == KeyCode::BackTab {
        let event_mods = event.modifiers & !KeyModifiers::SHIFT;
        let pattern_mods = pattern.modifiers & !KeyModifiers::SHIFT;
        return event_mods == pattern_mods;
    }

    event.modifiers == pattern.modifiers
}

/// Map an action string from the JSON keymap to an Action enum variant.
///
/// The mapping is exhaustive — every action string the keymap can produce
/// must be handled here. Unknown actions are logged and ignored (not a
/// crash — allows forward compatibility with newer keymaps).
fn map_action(action: &str) -> Option<Action> {
    match action {
        "Quit" => Some(Action::Quit),
        "ToggleHelp" => Some(Action::ToggleHelp),
        "FocusNext" => Some(Action::FocusNext),
        "FocusPrev" => Some(Action::FocusPrev),
        "ScrollDown" => Some(Action::ScrollDown(1)),
        "ScrollUp" => Some(Action::ScrollUp(1)),
        "ScrollToBottom" => Some(Action::ScrollToBottom),
        "ScrollToTop" => Some(Action::ScrollToTop),
        "ScrollPageDown" => Some(Action::ScrollPageDown),
        "ScrollPageUp" => Some(Action::ScrollPageUp),
        "Select" => Some(Action::Select),
        "Cancel" => Some(Action::Cancel),
        "Back" => Some(Action::Back),
        "NextTab" => Some(Action::NextTab),
        "PrevTab" => Some(Action::PrevTab),
        "OpenSearch" => Some(Action::OpenSearch(super::action::SearchMode::MessageSearch)),
        "OpenQuickSwitcher" => Some(Action::OpenQuickSwitcher),
        "Refresh" => Some(Action::Refresh),
        "ToggleSidebar" => Some(Action::ToggleSidebar),
        "EnterInputMode" => Some(Action::EnterInputMode),
        "ExitInputMode" => Some(Action::ExitInputMode),
        "InputSubmit" => Some(Action::InputSubmit),
        "OpenCommandPalette" => Some(Action::OpenSearch(
            super::action::SearchMode::CommandPalette,
        )),
        "ShowDashboard" => Some(Action::ShowDashboard),
        "ShowFriendList" => Some(Action::ShowFriendList),
        "ShowDmInbox" => Some(Action::ShowDmInbox),
        "ShowGroupDms" => Some(Action::ShowGroupDms),
        "ShowActivity" => Some(Action::ShowActivity),
        "ShowServers" => Some(Action::ShowServers),
        unknown => {
            tracing::warn!(action = unknown, "unknown keymap action — ignoring");
            None
        }
    }
}

#[cfg(test)]
#[path = "keybinds_tests.rs"]
mod tests;
