//! Combo string parsing and key event matching.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// A parsed key combination.
#[derive(Debug, Clone)]
pub struct KeyPattern {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
    pub display: String,
}

/// Parse a combo string like "Ctrl+k", "Shift+Tab", "Enter", "?" into a KeyPattern.
pub fn parse_combo(s: &str) -> anyhow::Result<KeyPattern> {
    let mut modifiers = KeyModifiers::empty();
    let mut remaining = s;

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
        s if s.len() == 1 => KeyCode::Char(s.chars().next().expect("non-empty")),
        other => anyhow::bail!("unknown key in combo '{s}': '{other}'"),
    };

    // Shift+Tab is BackTab in crossterm
    let code = if code == KeyCode::Tab && modifiers.contains(KeyModifiers::SHIFT) {
        modifiers.remove(KeyModifiers::SHIFT);
        KeyCode::BackTab
    } else {
        code
    };

    Ok(KeyPattern { code, modifiers, display: s.to_string() })
}

/// Check if a parsed KeyPattern matches a crossterm KeyEvent.
pub fn matches_key(pattern: &KeyPattern, event: &KeyEvent) -> bool {
    if pattern.code != event.code {
        return false;
    }

    // Handle uppercase chars — crossterm sets SHIFT for uppercase
    if let KeyCode::Char(c) = pattern.code {
        if c.is_ascii_uppercase() && !pattern.modifiers.contains(KeyModifiers::SHIFT) {
            let event_mods = event.modifiers & !KeyModifiers::SHIFT;
            let pattern_mods = pattern.modifiers & !KeyModifiers::SHIFT;
            return event_mods == pattern_mods;
        }
    }

    // BackTab: accept with or without SHIFT in event
    if pattern.code == KeyCode::BackTab {
        let event_mods = event.modifiers & !KeyModifiers::SHIFT;
        let pattern_mods = pattern.modifiers & !KeyModifiers::SHIFT;
        return event_mods == pattern_mods;
    }

    event.modifiers == pattern.modifiers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers, kind: KeyEventKind::Press, state: KeyEventState::empty() }
    }

    #[test]
    fn parse_simple_char() {
        let pat = parse_combo("q").unwrap();
        assert_eq!(pat.code, KeyCode::Char('q'));
        assert_eq!(pat.modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn parse_ctrl_modifier() {
        let pat = parse_combo("Ctrl+c").unwrap();
        assert_eq!(pat.code, KeyCode::Char('c'));
        assert!(pat.modifiers.contains(KeyModifiers::CONTROL));
    }

    #[test]
    fn parse_shift_tab_becomes_backtab() {
        let pat = parse_combo("Shift+Tab").unwrap();
        assert_eq!(pat.code, KeyCode::BackTab);
    }

    #[test]
    fn parse_special_keys() {
        assert_eq!(parse_combo("Enter").unwrap().code, KeyCode::Enter);
        assert_eq!(parse_combo("Esc").unwrap().code, KeyCode::Esc);
        assert_eq!(parse_combo("Space").unwrap().code, KeyCode::Char(' '));
        assert_eq!(parse_combo("PageUp").unwrap().code, KeyCode::PageUp);
    }

    #[test]
    fn parse_unknown_fails() {
        assert!(parse_combo("F13").is_err());
    }

    #[test]
    fn matches_uppercase_g() {
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
}
