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
    let action = store.classify(
        key(KeyCode::Char('q'), KeyModifiers::NONE),
        KeymapContext::Default,
    );
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
    let quit = store.classify(
        key(KeyCode::Char('q'), KeyModifiers::NONE),
        KeymapContext::Default,
    );
    assert!(matches!(quit, Some(Action::Quit)));

    // 'q' in Input context → None (not bound in Input)
    let none = store.classify(
        key(KeyCode::Char('q'), KeyModifiers::NONE),
        KeymapContext::Input,
    );
    assert!(none.is_none());
}

#[test]
fn classify_esc_cancels_in_non_input_contexts() {
    let store = KeymapStore::load().unwrap();

    // Esc maps to Cancel in Default, Overlay, and Search contexts
    for ctx in [
        KeymapContext::Default,
        KeymapContext::Overlay,
        KeymapContext::Search,
    ] {
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
    let action = store.classify(
        key(KeyCode::Tab, KeyModifiers::NONE),
        KeymapContext::Default,
    );
    assert!(matches!(action, Some(Action::FocusNext)));
}

#[test]
fn classify_shift_tab_focuses_prev() {
    let store = KeymapStore::load().unwrap();
    let action = store.classify(
        key(KeyCode::BackTab, KeyModifiers::NONE),
        KeymapContext::Default,
    );
    assert!(matches!(action, Some(Action::FocusPrev)));
}

#[test]
fn classify_enter_in_input_submits() {
    let store = KeymapStore::load().unwrap();
    let action = store.classify(
        key(KeyCode::Enter, KeyModifiers::NONE),
        KeymapContext::Input,
    );
    assert!(matches!(action, Some(Action::InputSubmit)));
}

#[test]
fn classify_unknown_key_returns_none() {
    let store = KeymapStore::load().unwrap();
    let action = store.classify(
        key(KeyCode::F(12), KeyModifiers::NONE),
        KeymapContext::Default,
    );
    assert!(action.is_none());
}

#[test]
fn help_text_returns_bindings_for_context() {
    let store = KeymapStore::load().unwrap();
    let help = store.help_text(KeymapContext::Default);
    assert!(!help.is_empty(), "Default context should have help entries");

    // Verify at least quit and help are present
    let descriptions: Vec<&str> = help.iter().map(|(_, d)| *d).collect();
    assert!(
        descriptions.contains(&"Quit"),
        "should contain Quit binding"
    );
    assert!(
        descriptions.contains(&"Toggle help"),
        "should contain help binding"
    );
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
    let mut seen: std::collections::HashMap<(String, String), String> =
        std::collections::HashMap::new();

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
    assert!(
        result.is_none(),
        "dispatch:false binding should not classify"
    );
}
