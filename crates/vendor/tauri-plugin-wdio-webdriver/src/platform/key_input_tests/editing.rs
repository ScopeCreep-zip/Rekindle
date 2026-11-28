use super::*;

#[test]
fn delete_keys_edit_text_fields_at_the_end_without_a_selection_api() {
    let none = mods([false, false, false, false]);
    let backspace = build_special_key_script("Backspace", "Backspace", 8, true, &none);
    assert!(backspace.contains(&format!("if ({})", is_editable_text_field_js("activeEl"))));
    let fallback = backspace.find("if (!hasSelectionApi) {").unwrap();
    let to_end = backspace
        .find("selStart = selEnd = currentValue.length;")
        .unwrap();
    assert!(fallback < to_end);
    assert!(backspace.contains("deleteContentBackward"));
    let delete = build_special_key_script("Delete", "Delete", 46, true, &none);
    assert!(delete.contains("deleteContentForward"));
}

#[test]
fn radio_navigation_escapes_the_group_name() {
    let script = build_special_key_script(
        "ArrowDown",
        "ArrowDown",
        40,
        true,
        &mods([false, false, false, false]),
    );
    assert!(script.contains("CSS.escape(activeEl.name)"));
    assert!(script.contains("activeEl.getRootNode().querySelectorAll("));
}

#[test]
fn segment_send_keys_splits_text_and_special_keys() {
    assert_eq!(
        segment_send_keys("hello"),
        vec![KeySegment::Text("hello".into())]
    );
    assert_eq!(segment_send_keys(""), vec![]);
    assert_eq!(
        segment_send_keys(ESCAPE),
        vec![KeySegment::Special(ESCAPE.into())]
    );

    assert_eq!(
        segment_send_keys(&format!("hi{ENTER}")),
        vec![
            KeySegment::Text("hi".into()),
            KeySegment::Special(ENTER.into())
        ]
    );

    assert_eq!(
        segment_send_keys(&format!("a{ARROW_DOWN}bc")),
        vec![
            KeySegment::Text("a".into()),
            KeySegment::Special(ARROW_DOWN.into()),
            KeySegment::Text("bc".into()),
        ]
    );

    assert_eq!(
        segment_send_keys(&format!("{ESCAPE}{ENTER}")),
        vec![
            KeySegment::Special(ESCAPE.into()),
            KeySegment::Special(ENTER.into())
        ]
    );
}

#[test]
fn segment_send_keys_never_leaves_a_special_key_in_text() {
    let everything: String = special_key_range().flat_map(|ch| ['x', ch]).collect();
    for segment in segment_send_keys(&everything) {
        if let KeySegment::Text(run) = segment {
            assert_eq!(run, "x");
        }
    }
}

#[test]
fn plan_send_keys_types_plain_text_in_one_step() {
    assert_eq!(plan_send_keys("hello"), vec![text("hello")]);
    assert_eq!(plan_send_keys(""), vec![]);
}

#[test]
fn plan_send_keys_drops_null_keys() {
    assert_eq!(plan_send_keys(&format!("a{NULL_KEY}b")), vec![text("ab")]);
    assert_eq!(plan_send_keys(NULL_KEY), vec![]);
}

#[test]
fn held_modifiers_start_from_keys_still_pressed() {
    let pressed = vec![
        SHIFT.to_string(),
        "a".to_string(),
        CONTROL_RIGHT.to_string(),
    ];
    let held = HeldModifiers::from_keys(&pressed);
    assert_eq!(held.state(), mods([true, true, false, false]));
    assert_eq!(held.keys(), vec![SHIFT, CONTROL_RIGHT]);
}

#[test]
fn held_modifiers_track_each_hand() {
    let mut held = HeldModifiers::default();
    assert!(held.press(CONTROL));
    assert!(held.press(CONTROL_RIGHT));
    assert!(!held.press(CONTROL));
    assert!(!held.press("a"));
    assert!(held.release(CONTROL_RIGHT));
    assert_eq!(held.state(), mods([true, false, false, false]));
    assert!(held.release(CONTROL));
    assert_eq!(held.state(), ModifierState::default());
    assert!(!held.release(CONTROL));
}

#[test]
fn held_modifiers_map_every_modifier_key() {
    let mut held = HeldModifiers::default();
    for key in ["\u{E050}", "\u{E052}", "\u{E053}"] {
        held.press(key);
    }
    assert_eq!(held.state(), mods([false, true, true, true]));
    assert_eq!(held.keys(), vec!["\u{E050}", "\u{E052}", "\u{E053}"]);
}

#[test]
fn plan_send_keys_presses_special_keys_between_text_runs() {
    let none = ModifierState::default();
    assert_eq!(
        plan_send_keys(&format!("abc{BACKSPACE}d{PAGE_DOWN}")),
        vec![
            text("abc"),
            key(BACKSPACE, true, none),
            key(BACKSPACE, false, none),
            text("d"),
            key(PAGE_DOWN, true, none),
            key(PAGE_DOWN, false, none),
        ]
    );
}

#[test]
fn plan_send_keys_holds_modifiers_until_the_end() {
    let none = ModifierState::default();
    let shift = mods([false, true, false, false]);
    assert_eq!(
        plan_send_keys(&format!("{SHIFT}ab{ENTER}")),
        vec![
            key(SHIFT, true, shift),
            text("ab"),
            key(ENTER, true, shift),
            key(ENTER, false, shift),
            key(SHIFT, false, none),
        ]
    );
}

#[test]
fn plan_send_keys_presses_text_as_shortcuts_under_control() {
    let none = ModifierState::default();
    let ctrl = mods([true, false, false, false]);
    assert_eq!(
        plan_send_keys(&format!("{CONTROL}a{BACKSPACE}")),
        vec![
            key(CONTROL, true, ctrl),
            key("a", true, ctrl),
            key("a", false, ctrl),
            key(BACKSPACE, true, ctrl),
            key(BACKSPACE, false, ctrl),
            key(CONTROL, false, none),
        ]
    );
}

#[test]
fn plan_send_keys_null_key_releases_modifiers() {
    let none = ModifierState::default();
    let ctrl = mods([true, false, false, false]);
    assert_eq!(
        plan_send_keys(&format!("{CONTROL}a{NULL_KEY}b")),
        vec![
            key(CONTROL, true, ctrl),
            key("a", true, ctrl),
            key("a", false, ctrl),
            key(CONTROL, false, none),
            text("b"),
        ]
    );
}

#[test]
fn plan_send_keys_toggles_a_repeated_modifier_and_releases_in_code_point_order() {
    let none = ModifierState::default();
    let alt = mods([false, false, true, false]);
    let shift = mods([false, true, false, false]);
    let alt_shift = mods([false, true, true, false]);
    let ctrl = mods([true, false, false, false]);
    let ctrl_shift = mods([true, true, false, false]);
    assert_eq!(
        plan_send_keys(&format!("{ALT}{SHIFT}{ALT}{CONTROL}")),
        vec![
            key(ALT, true, alt),
            key(SHIFT, true, alt_shift),
            key(ALT, false, shift),
            key(CONTROL, true, ctrl_shift),
            key(SHIFT, false, ctrl),
            key(CONTROL, false, none),
        ]
    );
}

#[test]
fn plan_send_keys_keeps_control_while_the_other_hand_holds_it() {
    let ctrl = mods([true, false, false, false]);
    let steps = plan_send_keys(&format!("{CONTROL}{CONTROL_RIGHT}a"));
    assert_eq!(steps[2], key("a", true, ctrl));
    assert_eq!(
        steps[4..],
        [
            key(CONTROL, false, ctrl),
            key(CONTROL_RIGHT, false, ModifierState::default()),
        ]
    );
}

#[test]
fn build_send_keys_start_script_appends_to_the_field_that_receives_the_text() {
    let script = build_send_keys_start_script("__wd_el_abc", "", None);
    let field_caret = script
        .find("el.setSelectionRange(el.value.length, el.value.length)")
        .unwrap();
    let focus = script.find("focusTarget.focus()").unwrap();
    let editor_caret = script.find("range.collapse(false)").unwrap();
    let delegated_caret = script
        .find("receiver.setSelectionRange(receiver.value.length, receiver.value.length)")
        .unwrap();
    assert!(field_caret < focus && focus < editor_caret && editor_caret < delegated_caret);
    assert!(script.contains("if (!wasFocused && el.isContentEditable)"));
    assert!(script.contains(&format!(
        "var initialValue = {} ? receiver.value : null;",
        is_text_field_js("receiver")
    )));
    assert!(script.contains("window.__wd_el_abc__receiver = receiver;"));
    let plain = build_send_keys_start_script("__wd_el_abc", "x", Some("x"));
    assert!(plain.contains("receiver.value !== initialValue"));
    assert!(plain.contains("receiver.dispatchEvent(new Event('change'"));
}

#[test]
fn build_send_keys_start_script_accepts_the_body_editing_hosts_and_focus_inside() {
    let script = build_send_keys_start_script("__wd_el_abc", "", None);
    assert!(script.contains(
        "el === document.body || el === document.documentElement || activeEl === focusTarget;"
    ));
    assert!(script.contains("focusTarget = focusTarget.parentElement;"));
    assert!(script.contains("node = node.parentNode || node.host"));
    assert!(script.contains("return { status: 'unfocused' };"));
}

#[test]
fn build_send_keys_start_script_reports_a_stale_element_without_throwing() {
    let script = build_send_keys_start_script("__wd_el_abc", "x", Some("x"));
    assert!(script.contains("return { status: 'stale' };"));
    assert!(!script.contains("throw"));
}

#[test]
fn build_send_keys_start_script_types_plain_text_in_the_same_script() {
    let plain = build_send_keys_start_script("__wd_el_abc", "hi", Some("hi"));
    let focus = plain.find("focusTarget.focus()").unwrap();
    let insert = plain.find("var insertedText = \"hi\"").unwrap();
    let change = plain.rfind("new Event('change'").unwrap();
    assert!(focus < insert && insert < change);
    assert!(plain.contains("return { status: 'done' };"));
    let empty = build_send_keys_start_script("__wd_el_abc", "", Some(""));
    assert!(!empty.contains("insertedText") && empty.contains("return { status: 'done' };"));
}

#[test]
fn build_insert_run_script_inserts_fires_input_and_defers_change() {
    let script = build_insert_run_script("hi");
    assert!(script.contains("var activeEl = document.activeElement || document.body;"));
    assert!(script.contains("var insertedText = \"hi\""));
    assert!(script.contains("inputType: 'insertText'"));
    assert!(script.contains("document.execCommand('insertText', false, insertedText)"));
    assert!(!script.contains("new Event('change'"));
}

#[test]
fn build_send_keys_change_script_fires_change_only_for_a_changed_value() {
    let script = build_send_keys_change_script("__wd_el_abc", "it's\n");
    assert!(script.contains("var el = window.__wd_el_abc__receiver;"));
    assert!(script.contains(&format!("el.value !== {}", js_string("it's\n"))));
    assert!(script.contains("new Event('change'"));
    assert!(!script.contains("throw"));
}

#[test]
fn key_and_input_events_cross_shadow_boundaries() {
    let none = mods([false, false, false, false]);
    for script in [
        build_special_key_script("Backspace", "Backspace", 8, true, &none),
        build_printable_key_script(" ", "Space", 32, true, &none),
    ] {
        let keyboard = script.find("new KeyboardEvent(").unwrap();
        let input = script.find("new InputEvent('input'").unwrap();
        assert!(script[keyboard..].contains("composed: true"));
        assert!(script[input..].contains("composed: true"));
        assert_eq!(script.matches("composed: true").count(), 2, "{script}");
    }
    assert!(build_special_key_script("End", "End", 35, false, &none).contains("composed: true"));
}

#[test]
fn typing_skips_read_only_and_disabled_fields() {
    let none = mods([false, false, false, false]);
    let editable = is_editable_text_field_js("activeEl");
    assert!(editable.contains("!activeEl.readOnly && !activeEl.disabled"));
    for script in [
        build_printable_key_script("x", "KeyX", 120, true, &none),
        build_special_key_script("Enter", "Enter", 13, true, &none),
        build_special_key_script("Backspace", "Backspace", 8, true, &none),
        build_special_key_script("Delete", "Delete", 46, true, &none),
    ] {
        assert!(script.contains(&format!("if ({editable})")), "{script}");
    }
    assert!(build_insert_run_script("x")
        .contains(&format!("if ({})", is_editable_text_field_js("activeEl"))));
}

#[test]
fn value_controls_get_their_value_set_directly() {
    let control = is_value_control_js("el");
    for kind in [
        "color",
        "date",
        "datetime-local",
        "month",
        "range",
        "time",
        "week",
    ] {
        assert!(control.contains(&format!("'{kind}'")));
    }
    for typeable in ["text", "checkbox", "radio", "file", "button"] {
        assert!(!control.contains(&format!("'{typeable}'")));
    }
    let script = build_send_keys_start_script("__wd_el_abc", "2026-10-21", Some("2026-10-21"));
    assert!(script.contains(&format!("var typed = {};", js_string("2026-10-21"))));
    assert!(script.contains(&format!("if ({control} && typed !== '')")));
    assert!(script.contains("return { status: 'immutable' };"));
    let input = script.find("new Event('input'").unwrap();
    let change = script.find("new Event('change'").unwrap();
    assert!(script.contains("if (el.value !== previousValue)") && input < change);
}

#[test]
fn send_keys_start_refuses_file_inputs_and_invalid_values() {
    let script = build_send_keys_start_script("__wd_el_abc", "not-a-date", Some("not-a-date"));
    let file = script.find("el.type === 'file'").unwrap();
    assert!(file < script.find("var typed").unwrap());
    let invalid = script.find("if (el.value === '') {").unwrap();
    let restore = script.find("setValue.call(el, previousValue);").unwrap();
    assert!(invalid < restore && restore < script.find("status: 'invalid'").unwrap());
}

#[test]
fn typed_text_keeps_only_the_text_runs() {
    let steps = plan_send_keys(&format!("20{BACKSPACE}26-10{SHIFT}-21"));
    assert_eq!(typed_text(&steps), "2026-10-21");
    assert_eq!(typed_text(&plan_send_keys(NULL_KEY)), "");
}
