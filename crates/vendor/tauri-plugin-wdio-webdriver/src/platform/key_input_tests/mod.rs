use super::*;
use crate::platform::send_keys::{segment_send_keys, KeySegment};

mod editing;

const ESCAPE: &str = "\u{E00C}";
const ENTER: &str = "\u{E007}";
const BACKSPACE: &str = "\u{E003}";
const PAGE_DOWN: &str = "\u{E00F}";
const ARROW_DOWN: &str = "\u{E015}";
const SHIFT: &str = "\u{E008}";
const CONTROL: &str = "\u{E009}";
const CONTROL_RIGHT: &str = "\u{E051}";
const ALT: &str = "\u{E00A}";

fn special_key_range() -> impl Iterator<Item = char> {
    '\u{E000}'..='\u{E05D}'
}

fn mods([ctrl, shift, alt, meta]: [bool; 4]) -> ModifierState {
    ModifierState {
        ctrl,
        shift,
        alt,
        meta,
    }
}

fn key(value: &str, is_down: bool, modifiers: ModifierState) -> SendKeysStep {
    SendKeysStep::Key {
        value: value.to_string(),
        is_down,
        modifiers,
    }
}

fn text(run: &str) -> SendKeysStep {
    SendKeysStep::Text(run.to_string())
}

#[test]
fn every_special_key_code_point_maps() {
    for ch in special_key_range() {
        assert!(
            map_special_key(&ch.to_string()).is_some(),
            "U+{:04X} has no mapping",
            ch as u32
        );
    }
}

#[test]
fn no_mapping_leaks_a_private_use_character() {
    for ch in special_key_range() {
        let (key, code, _) = map_special_key(&ch.to_string()).unwrap();
        assert!(
            !key.chars()
                .chain(code.chars())
                .any(|c| ('\u{E000}'..='\u{E05D}').contains(&c)),
            "U+{:04X} maps to a private-use value",
            ch as u32
        );
    }
}

#[test]
fn map_special_key_maps_the_reported_navigation_keys() {
    assert_eq!(map_special_key("\u{E00E}"), Some(("PageUp", "PageUp", 33)));
    assert_eq!(
        map_special_key(PAGE_DOWN),
        Some(("PageDown", "PageDown", 34))
    );
    assert_eq!(map_special_key("\u{E010}"), Some(("End", "End", 35)));
    assert_eq!(map_special_key("\u{E011}"), Some(("Home", "Home", 36)));
    assert_eq!(map_special_key("\u{E016}"), Some(("Insert", "Insert", 45)));
}

#[test]
fn map_special_key_keeps_the_existing_entries() {
    assert_eq!(map_special_key(ESCAPE), Some(("Escape", "Escape", 27)));
    assert_eq!(map_special_key(ENTER), Some(("Enter", "Enter", 13)));
    assert_eq!(
        map_special_key("\u{E006}"),
        Some(("Enter", "NumpadEnter", 13))
    );
    assert_eq!(map_special_key("\u{E00D}"), Some((" ", "Space", 32)));
    assert_eq!(
        map_special_key(ARROW_DOWN),
        Some(("ArrowDown", "ArrowDown", 40))
    );
    assert_eq!(map_special_key("\u{E03C}"), Some(("F12", "F12", 123)));
    assert_eq!(
        map_special_key(CONTROL),
        Some(("Control", "ControlLeft", 17))
    );
}

#[test]
fn map_special_key_maps_numpad_and_right_hand_keys() {
    assert_eq!(map_special_key("\u{E01A}"), Some(("0", "Numpad0", 96)));
    assert_eq!(map_special_key("\u{E023}"), Some(("9", "Numpad9", 105)));
    assert_eq!(
        map_special_key("\u{E029}"),
        Some(("/", "NumpadDivide", 111))
    );
    assert_eq!(map_special_key("\u{E019}"), Some(("=", "NumpadEqual", 187)));
    assert_eq!(map_special_key("\u{E018}"), Some((";", "Semicolon", 186)));
    assert_eq!(
        map_special_key("\u{E050}"),
        Some(("Shift", "ShiftRight", 16))
    );
    assert_eq!(map_special_key("\u{E053}"), Some(("Meta", "MetaRight", 92)));
    assert_eq!(map_special_key("\u{E054}"), Some(("PageUp", "Numpad9", 33)));
    assert_eq!(
        map_special_key("\u{E05D}"),
        Some(("Delete", "NumpadDecimal", 46))
    );
}

#[test]
fn map_special_key_maps_the_remaining_named_keys() {
    assert_eq!(map_special_key(NULL_KEY), Some(("Unidentified", "", 0)));
    assert_eq!(map_special_key("\u{E001}"), Some(("Cancel", "", 3)));
    assert_eq!(map_special_key("\u{E002}"), Some(("Help", "Help", 47)));
    assert_eq!(map_special_key("\u{E005}"), Some(("Clear", "", 12)));
    assert_eq!(map_special_key("\u{E00B}"), Some(("Pause", "Pause", 19)));
    assert_eq!(map_special_key("\u{E040}"), Some(("ZenkakuHankaku", "", 0)));
}

#[test]
fn unassigned_code_points_map_to_unidentified() {
    for unassigned in ["\u{E02A}", "\u{E030}", "\u{E03E}", "\u{E041}", "\u{E04F}"] {
        assert_eq!(map_special_key(unassigned), Some(("Unidentified", "", 0)));
    }
}

#[test]
fn map_special_key_rejects_everything_outside_the_range() {
    assert_eq!(map_special_key("a"), None);
    assert_eq!(map_special_key("5"), None);
    assert_eq!(map_special_key(""), None);
    assert_eq!(map_special_key("\u{D7FF}"), None);
    assert_eq!(map_special_key("\u{E05E}"), None);
    assert_eq!(map_special_key("\u{F8FF}"), None);
    assert_eq!(map_special_key(&format!("{PAGE_DOWN}{PAGE_DOWN}")), None);
}

#[test]
fn printable_special_keys_are_single_characters() {
    let printable: Vec<char> = special_key_range()
        .filter(|ch| is_printable_key(map_special_key(&ch.to_string()).unwrap().0))
        .collect();
    let expected: Vec<char> = std::iter::once('\u{E00D}')
        .chain('\u{E018}'..='\u{E029}')
        .collect();
    assert_eq!(printable, expected);
}

#[test]
fn is_modifier_key_covers_both_hands() {
    for modifier in [
        SHIFT, CONTROL, ALT, "\u{E03D}", "\u{E050}", "\u{E051}", "\u{E052}", "\u{E053}",
    ] {
        assert!(is_modifier_key(modifier));
    }
    assert!(!is_modifier_key(ENTER));
    assert!(!is_modifier_key("a"));
}

#[test]
fn regular_key_code_for_derives_code() {
    assert_eq!(regular_key_code_for("a"), "KeyA");
    assert_eq!(regular_key_code_for("A"), "KeyA");
    assert_eq!(regular_key_code_for("5"), "Digit5");
    assert_eq!(regular_key_code_for("!"), "!");
}

#[test]
fn build_special_key_script_dispatches_keyboard_event() {
    let none = ModifierState::default();
    let script = build_special_key_script("PageDown", "PageDown", 34, true, &none);
    assert!(script.contains("new KeyboardEvent('keydown'"));
    assert!(script.contains("key: \"PageDown\""));
    assert!(script.contains("keyCode: 34"));
    assert!(script.contains("shiftKey: false"));
    assert_eq!(script.matches("new KeyboardEvent(").count(), 1);

    let up = build_special_key_script("PageDown", "PageDown", 34, false, &none);
    assert!(up.contains("new KeyboardEvent('keyup'"));
}

#[test]
fn build_special_key_script_carries_held_modifiers_on_every_event() {
    let shift = mods([false, true, false, false]);
    let ctrl_alt_meta = mods([true, false, true, true]);
    for script in [
        build_special_key_script("Enter", "Enter", 13, true, &shift),
        build_special_key_script("Enter", "Enter", 13, false, &shift),
        build_special_key_script("Backspace", "Backspace", 8, true, &shift),
        build_special_key_script("ArrowDown", "ArrowDown", 40, true, &shift),
    ] {
        assert!(script.contains("shiftKey: true"), "{script}");
        assert!(script.contains("ctrlKey: false"), "{script}");
    }
    let chord = build_special_key_script("Home", "Home", 36, true, &ctrl_alt_meta);
    assert!(chord.contains("ctrlKey: true"));
    assert!(chord.contains("altKey: true"));
    assert!(chord.contains("metaKey: true"));
    assert!(chord.contains("shiftKey: false"));
}

#[test]
fn build_regular_key_script_carries_modifiers_and_value_mutation() {
    let script = build_regular_key_script("a", "KeyA", true, &mods([false, false, false, false]));
    assert!(script.contains("key: \"a\""));
    assert!(script.contains("keyCode: 97"));
    assert!(script.contains("inputType: 'insertText'"));

    let select_all =
        build_regular_key_script("a", "KeyA", true, &mods([true, false, false, false]));
    assert!(select_all.contains("activeEl.select()"));
    assert!(!select_all.contains("insertedText"));
}

#[test]
fn build_printable_key_script_uses_the_given_key_code() {
    let script = build_printable_key_script(
        "5",
        "Numpad5",
        101,
        true,
        &mods([false, false, false, false]),
    );
    assert!(script.contains("key: \"5\""));
    assert!(script.contains("code: \"Numpad5\""));
    assert!(script.contains("keyCode: 101"));
    assert!(script.contains("var insertedText = \"5\""));
}

#[test]
fn build_printable_key_script_types_into_contenteditable() {
    let script =
        build_printable_key_script(" ", "Space", 32, true, &mods([false, false, false, false]));
    assert!(script.contains("activeEl.isContentEditable"));
    assert!(script.contains("document.execCommand('insertText', false, insertedText)"));
}

#[test]
fn typed_text_replaces_the_selection() {
    let none = mods([false, false, false, false]);
    for script in [
        build_printable_key_script("5", "Numpad5", 101, true, &none),
        build_insert_run_script("new"),
    ] {
        assert!(script.contains(
            "currentValue.slice(0, selStart) + insertedText + currentValue.slice(selEnd)"
        ));
        assert!(script.contains("setSelectionRange(caret, caret)"));
        assert!(script.contains("if (activeEl.value !== currentValue)"));
    }
}

#[test]
fn only_text_fields_and_editors_take_typed_text() {
    let field = is_text_field_js("el");
    assert!(field.contains("el.tagName === 'TEXTAREA'"));
    assert!(field.contains(
        "['text', 'search', 'url', 'tel', 'email', 'password', 'number'].indexOf(el.type)"
    ));
    let script = build_insert_run_script("x");
    assert!(script.contains(&format!("if ({})", is_editable_text_field_js("activeEl"))));
    assert!(is_editable_text_field_js("el").contains(&field));
    assert!(!script.contains("el.tagName === 'INPUT' || el.tagName === 'TEXTAREA'"));
}

#[test]
fn js_string_escapes_line_breaks_and_quotes() {
    let text = "a'\"\\\n";
    let literal = js_string(text);
    assert!(literal.starts_with('"') && literal.ends_with('"'));
    assert!(!literal.contains('\n'));
    assert_eq!(serde_json::from_str::<String>(&literal).unwrap(), text);
}

#[test]
fn key_text_is_spliced_as_escaped_literals() {
    let none = mods([false, false, false, false]);
    let newline = build_regular_key_script("\n", "\n", true, &none);
    assert!(newline.contains(&format!("key: {}", js_string("\n"))));
    assert!(newline.contains(&format!("var insertedText = {}", js_string("\n"))));
    let run = build_insert_run_script("a`$\\b\n'");
    assert!(run.contains(&format!("var insertedText = {}", js_string("a`$\\b\n'"))));
}

#[test]
fn key_scripts_target_the_deep_active_element() {
    let none = mods([false, false, false, false]);
    for script in [
        build_special_key_script("Enter", "Enter", 13, true, &none),
        build_printable_key_script(" ", "Space", 32, true, &none),
        build_insert_run_script("x"),
        build_send_keys_start_script("v", "", None),
    ] {
        assert!(
            script.contains("activeEl.shadowRoot.activeElement"),
            "{script}"
        );
    }
}

#[test]
fn default_actions_wait_for_an_uncanceled_keydown() {
    let none = mods([false, false, false, false]);
    let ctrl = mods([true, false, false, false]);
    let guard = "if (!activeEl.dispatchEvent(keyEvent)) {";
    for (script, side_effect) in [
        (
            build_printable_key_script(" ", "Space", 32, true, &none),
            "var insertedText",
        ),
        (
            build_regular_key_script("x", "KeyX", true, &none),
            "var insertedText",
        ),
        (
            build_regular_key_script("a", "KeyA", true, &ctrl),
            "activeEl.select()",
        ),
        (
            build_special_key_script("Backspace", "Backspace", 8, true, &none),
            "nativeInputValueSetter",
        ),
        (
            build_special_key_script("Delete", "Delete", 46, true, &none),
            "nativeInputValueSetter",
        ),
        (
            build_special_key_script("ArrowDown", "ArrowDown", 40, true, &none),
            "nextRadio.checked",
        ),
        (
            build_special_key_script("Enter", "Enter", 13, true, &none),
            "insertParagraph",
        ),
    ] {
        let guard_at = script
            .find(guard)
            .unwrap_or_else(|| panic!("no guard in {script}"));
        let effect_at = script.find(side_effect).unwrap();
        assert!(
            guard_at < effect_at,
            "{side_effect} runs before the guard in {script}"
        );
        assert!(!script.contains("activeEl.dispatchEvent(keyEvent);"));
    }
}

#[test]
fn enter_breaks_lines_in_textareas_and_editors_only() {
    let none = mods([false, false, false, false]);
    let shift = mods([false, true, false, false]);
    let enter = build_special_key_script("Enter", "Enter", 13, true, &none);
    assert!(enter.contains("if (activeEl.tagName === 'TEXTAREA')"));
    assert!(enter.contains(&format!("var insertedText = {}", js_string("\n"))));
    assert!(enter.contains("document.execCommand('insertParagraph', false, null)"));
    let shift_enter = build_special_key_script("Enter", "Enter", 13, true, &shift);
    assert!(shift_enter.contains("document.execCommand('insertLineBreak', false, null)"));
    let enter_up = build_special_key_script("Enter", "Enter", 13, false, &none);
    assert!(!enter_up.contains("insertedText"));
    for chord in [
        mods([true, false, false, false]),
        mods([false, false, true, false]),
        mods([false, false, false, true]),
    ] {
        let script = build_special_key_script("Enter", "Enter", 13, true, &chord);
        assert!(!script.contains("insertedText") && !script.contains("insertParagraph"));
    }
}
