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
const KEYMAP_JSON: &str = include_str!("../../keymap/default.keymap.json");

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
        "OpenCommandPalette" => Some(Action::OpenSearch(super::action::SearchMode::CommandPalette)),
        "ShowDashboard" => Some(Action::ShowDashboard),
        "ShowFriendList" => Some(Action::ShowFriendList),
        "ShowDmInbox" => Some(Action::ShowDmInbox),
        unknown => {
            tracing::warn!(action = unknown, "unknown keymap action — ignoring");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::empty(),
        }
    }

    #[test]
    fn load_succeeds() {
        let store = KeymapStore::load().expect("keymap should parse");
        assert!(!store.bindings.is_empty());
    }

    #[test]
    fn classify_q_quits() {
        let store = KeymapStore::load().unwrap();
        let action = store.classify(key(KeyCode::Char('q'), KeyModifiers::NONE), KeymapContext::Default);
        assert!(matches!(action, Some(Action::Quit)));
    }

    #[test]
    fn classify_ctrl_c_quits() {
        let store = KeymapStore::load().unwrap();
        let action = store.classify(
            key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            KeymapContext::Default,
        );
        assert!(matches!(action, Some(Action::Quit)));
    }

    #[test]
    fn classify_respects_context() {
        let store = KeymapStore::load().unwrap();

        // 'q' in Default context → Quit
        let quit = store.classify(key(KeyCode::Char('q'), KeyModifiers::NONE), KeymapContext::Default);
        assert!(matches!(quit, Some(Action::Quit)));

        // 'q' in Input context → None (not bound in Input)
        let none = store.classify(key(KeyCode::Char('q'), KeyModifiers::NONE), KeymapContext::Input);
        assert!(none.is_none());
    }

    #[test]
    fn classify_esc_cancels_in_non_input_contexts() {
        let store = KeymapStore::load().unwrap();

        // Esc maps to Cancel in Default, Overlay, and Search contexts
        for ctx in [KeymapContext::Default, KeymapContext::Overlay, KeymapContext::Search] {
            let action = store.classify(key(KeyCode::Esc, KeyModifiers::NONE), ctx);
            assert!(
                matches!(action, Some(Action::Cancel)),
                "Esc should map to Cancel in {ctx:?}"
            );
        }

        // In Input context, Esc maps to ExitInputMode (not Cancel)
        // because input.exit binds Esc in Input context exclusively
        let action = store.classify(key(KeyCode::Esc, KeyModifiers::NONE), KeymapContext::Input);
        assert!(
            matches!(action, Some(Action::ExitInputMode)),
            "Esc should map to ExitInputMode in Input context"
        );
    }

    #[test]
    fn classify_tab_focuses_next() {
        let store = KeymapStore::load().unwrap();
        let action = store.classify(key(KeyCode::Tab, KeyModifiers::NONE), KeymapContext::Default);
        assert!(matches!(action, Some(Action::FocusNext)));
    }

    #[test]
    fn classify_shift_tab_focuses_prev() {
        let store = KeymapStore::load().unwrap();
        let action = store.classify(key(KeyCode::BackTab, KeyModifiers::NONE), KeymapContext::Default);
        assert!(matches!(action, Some(Action::FocusPrev)));
    }

    #[test]
    fn classify_enter_in_input_submits() {
        let store = KeymapStore::load().unwrap();
        let action = store.classify(key(KeyCode::Enter, KeyModifiers::NONE), KeymapContext::Input);
        assert!(matches!(action, Some(Action::InputSubmit)));
    }

    #[test]
    fn classify_unknown_key_returns_none() {
        let store = KeymapStore::load().unwrap();
        let action = store.classify(key(KeyCode::F(12), KeyModifiers::NONE), KeymapContext::Default);
        assert!(action.is_none());
    }

    #[test]
    fn help_text_returns_bindings_for_context() {
        let store = KeymapStore::load().unwrap();
        let help = store.help_text(KeymapContext::Default);
        assert!(!help.is_empty(), "Default context should have help entries");

        // Verify at least quit and help are present
        let descriptions: Vec<&str> = help.iter().map(|(_, d)| *d).collect();
        assert!(descriptions.contains(&"Quit"), "should contain Quit binding");
        assert!(descriptions.contains(&"Toggle help"), "should contain help binding");
    }

    #[test]
    fn help_text_input_context_has_submit() {
        let store = KeymapStore::load().unwrap();
        let help = store.help_text(KeymapContext::Input);
        let descriptions: Vec<&str> = help.iter().map(|(_, d)| *d).collect();
        assert!(descriptions.contains(&"Send message"));
    }

    #[test]
    fn hint_line_produces_compact_string() {
        let store = KeymapStore::load().unwrap();
        let line = store.hint_line(KeymapContext::Default);
        assert!(line.contains("Quit"), "hint should contain Quit: {line}");
        assert!(line.contains('|'), "hints should be pipe-separated: {line}");
    }

    #[test]
    fn parse_combo_simple_char() {
        let pat = parse_combo("q").unwrap();
        assert_eq!(pat.code, KeyCode::Char('q'));
        assert_eq!(pat.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn parse_combo_ctrl() {
        let pat = parse_combo("Ctrl+c").unwrap();
        assert_eq!(pat.code, KeyCode::Char('c'));
        assert!(pat.modifiers.contains(KeyModifiers::CONTROL));
    }

    #[test]
    fn parse_combo_shift_tab() {
        let pat = parse_combo("Shift+Tab").unwrap();
        assert_eq!(pat.code, KeyCode::BackTab);
        // SHIFT is removed because BackTab already implies it
        assert!(!pat.modifiers.contains(KeyModifiers::SHIFT));
    }

    #[test]
    fn parse_combo_special_keys() {
        assert_eq!(parse_combo("Enter").unwrap().code, KeyCode::Enter);
        assert_eq!(parse_combo("Esc").unwrap().code, KeyCode::Esc);
        assert_eq!(parse_combo("Tab").unwrap().code, KeyCode::Tab);
        assert_eq!(parse_combo("Space").unwrap().code, KeyCode::Char(' '));
        assert_eq!(parse_combo("Home").unwrap().code, KeyCode::Home);
        assert_eq!(parse_combo("End").unwrap().code, KeyCode::End);
        assert_eq!(parse_combo("PageUp").unwrap().code, KeyCode::PageUp);
        assert_eq!(parse_combo("PageDown").unwrap().code, KeyCode::PageDown);
    }

    #[test]
    fn parse_combo_unknown_key_fails() {
        assert!(parse_combo("F13").is_err());
        assert!(parse_combo("SuperKey").is_err());
    }

    #[test]
    fn matches_uppercase_g() {
        // 'G' in keymap means the literal uppercase G character.
        // crossterm reports Char('G') with SHIFT modifier.
        let pat = parse_combo("G").unwrap();
        let evt = key(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert!(matches_key(&pat, &evt));
    }

    #[test]
    fn matches_ctrl_k() {
        let pat = parse_combo("Ctrl+k").unwrap();
        let evt = key(KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert!(matches_key(&pat, &evt));
    }

    #[test]
    fn no_duplicate_bindings_in_same_context() {
        let store = KeymapStore::load().unwrap();
        let mut seen: std::collections::HashMap<(String, String), String> = std::collections::HashMap::new();

        for binding in &store.bindings {
            if !binding.dispatch {
                continue; // help-only entries can share combos
            }
            for context in &binding.contexts {
                for combo in &binding.combos {
                    let key = (format!("{context:?}"), combo.display.clone());
                    if let Some(existing_id) = seen.get(&key) {
                        panic!(
                            "keybinding conflict: '{}' and '{}' both bind '{}' in {:?} context",
                            existing_id, binding.id, combo.display, context
                        );
                    }
                    seen.insert(key, binding.id.clone());
                }
            }
        }
    }

    #[test]
    fn non_dispatch_bindings_excluded_from_classify() {
        let store = KeymapStore::load().unwrap();
        // Shift+Enter is dispatch:false — should not classify to any action
        let result = store.classify(
            key(KeyCode::Enter, KeyModifiers::SHIFT),
            KeymapContext::Input,
        );
        assert!(result.is_none(), "dispatch:false binding should not classify");
    }
}
