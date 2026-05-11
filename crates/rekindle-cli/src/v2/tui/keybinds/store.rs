//! KeymapStore — load, classify, help text, hint line.

use crossterm::event::KeyEvent;

use super::context::KeymapContext;
use super::map::map_action;
use super::parse::{parse_combo, matches_key, KeyPattern};
use super::super::action::Action;

/// The embedded keymap JSON — compiled into the binary.
const KEYMAP_JSON: &str = include_str!("../../../../keymap/default.keymap.json");

/// A single keybinding entry parsed from JSON.
struct KeyBinding {
    id: String,
    description: String,
    contexts: Vec<KeymapContext>,
    action: String,
    combos: Vec<KeyPattern>,
    dispatch: bool,
}

/// JSON deserialization shape.
#[derive(serde::Deserialize)]
struct RawBinding {
    id: String,
    description: String,
    contexts: Vec<KeymapContext>,
    action: String,
    combos: Vec<String>,
    dispatch: Option<bool>,
}

/// Immutable keymap store — loaded once at startup.
pub struct KeymapStore {
    bindings: Vec<KeyBinding>,
}

impl KeymapStore {
    /// Parse the embedded JSON keymap.
    pub fn load() -> anyhow::Result<Self> {
        let raw: Vec<RawBinding> = serde_json::from_str(KEYMAP_JSON)
            .map_err(|e| anyhow::anyhow!("keymap parse failed: {e}"))?;

        let mut bindings = Vec::with_capacity(raw.len());
        for entry in raw {
            let combos: Vec<KeyPattern> = entry.combos
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
    /// First match wins. Returns None if no binding matches.
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

    /// Help text for a context: (display_combo, description) pairs.
    pub fn help_text(&self, context: KeymapContext) -> Vec<(&str, &str)> {
        self.bindings
            .iter()
            .filter(|b| b.contexts.contains(&context))
            .filter_map(|b| {
                b.combos.first().map(|c| (c.display.as_str(), b.description.as_str()))
            })
            .collect()
    }

    /// Compact hint string for the status bar.
    pub fn hint_line(&self, context: KeymapContext) -> String {
        self.help_text(context)
            .iter()
            .take(6)
            .map(|(combo, desc)| format!("{combo} {desc}"))
            .collect::<Vec<_>>()
            .join(" | ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers, kind: KeyEventKind::Press, state: KeyEventState::empty() }
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
        let action = store.classify(key(KeyCode::Char('c'), KeyModifiers::CONTROL), KeymapContext::Default);
        assert!(matches!(action, Some(Action::Quit)));
    }

    #[test]
    fn classify_respects_context() {
        let store = KeymapStore::load().unwrap();
        assert!(matches!(
            store.classify(key(KeyCode::Char('q'), KeyModifiers::NONE), KeymapContext::Default),
            Some(Action::Quit)
        ));
        assert!(store.classify(key(KeyCode::Char('q'), KeyModifiers::NONE), KeymapContext::Input).is_none());
    }

    #[test]
    fn classify_esc_in_input_exits() {
        let store = KeymapStore::load().unwrap();
        let action = store.classify(key(KeyCode::Esc, KeyModifiers::NONE), KeymapContext::Input);
        assert!(matches!(action, Some(Action::ExitInputMode)));
    }

    #[test]
    fn classify_enter_in_input_submits() {
        let store = KeymapStore::load().unwrap();
        let action = store.classify(key(KeyCode::Enter, KeyModifiers::NONE), KeymapContext::Input);
        assert!(matches!(action, Some(Action::InputSubmit)));
    }

    #[test]
    fn help_text_nonempty() {
        let store = KeymapStore::load().unwrap();
        let help = store.help_text(KeymapContext::Default);
        assert!(!help.is_empty());
    }

    #[test]
    fn hint_line_contains_quit() {
        let store = KeymapStore::load().unwrap();
        let line = store.hint_line(KeymapContext::Default);
        assert!(line.contains("Quit"));
    }

    #[test]
    fn no_duplicate_bindings_in_same_context() {
        let store = KeymapStore::load().unwrap();
        let mut seen: std::collections::HashMap<(String, String), String> = std::collections::HashMap::new();

        for binding in &store.bindings {
            if !binding.dispatch { continue; }
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
}
