//! Keyboard input mapping — shared, platform-agnostic building blocks.

use super::executor::ModifierState;

// Element send-keys planning lives in `send_keys`; re-exported so callers keep one path.
pub use super::send_keys::{
    build_insert_run_script, build_send_keys_change_script, build_send_keys_start_script,
    plain_text, plan_send_keys, typed_text, HeldModifiers, SendKeysStep,
};

/// The WebDriver null key; in Element Send Keys it releases every held modifier.
pub const NULL_KEY: &str = "\u{E000}";

/// Maps every WebDriver special-key value (U+E000–U+E05D) to `(key, code, keyCode)` per the W3C
/// tables and a US keyboard, so none is ever typed as text.
pub fn map_special_key(key: &str) -> Option<(&'static str, &'static str, u32)> {
    let mut chars = key.chars();
    let (Some(ch), None) = (chars.next(), chars.next()) else {
        return None;
    };
    let mapped = match ch {
        '\u{E001}' => ("Cancel", "", 3),
        '\u{E002}' => ("Help", "Help", 47),
        '\u{E003}' => ("Backspace", "Backspace", 8),
        '\u{E004}' => ("Tab", "Tab", 9),
        '\u{E005}' => ("Clear", "", 12),
        // The spec's code table puts the main Enter key on U+E006 ("Return") and NumpadEnter on
        // U+E007, but clients send U+E007 for Enter, so it keeps the main key's code.
        '\u{E006}' => ("Enter", "NumpadEnter", 13),
        '\u{E007}' => ("Enter", "Enter", 13),
        '\u{E008}' => ("Shift", "ShiftLeft", 16),
        '\u{E009}' => ("Control", "ControlLeft", 17),
        '\u{E00A}' => ("Alt", "AltLeft", 18),
        '\u{E00B}' => ("Pause", "Pause", 19),
        '\u{E00C}' => ("Escape", "Escape", 27),
        '\u{E00D}' => (" ", "Space", 32),
        '\u{E00E}' => ("PageUp", "PageUp", 33),
        '\u{E00F}' => ("PageDown", "PageDown", 34),
        '\u{E010}' => ("End", "End", 35),
        '\u{E011}' => ("Home", "Home", 36),
        '\u{E012}' => ("ArrowLeft", "ArrowLeft", 37),
        '\u{E013}' => ("ArrowUp", "ArrowUp", 38),
        '\u{E014}' => ("ArrowRight", "ArrowRight", 39),
        '\u{E015}' => ("ArrowDown", "ArrowDown", 40),
        '\u{E016}' => ("Insert", "Insert", 45),
        '\u{E017}' => ("Delete", "Delete", 46),
        // The spec's code table has no row for U+E018; this is the US-layout `;` key.
        '\u{E018}' => (";", "Semicolon", 186),
        '\u{E019}' => ("=", "NumpadEqual", 187),
        '\u{E01A}' => ("0", "Numpad0", 96),
        '\u{E01B}' => ("1", "Numpad1", 97),
        '\u{E01C}' => ("2", "Numpad2", 98),
        '\u{E01D}' => ("3", "Numpad3", 99),
        '\u{E01E}' => ("4", "Numpad4", 100),
        '\u{E01F}' => ("5", "Numpad5", 101),
        '\u{E020}' => ("6", "Numpad6", 102),
        '\u{E021}' => ("7", "Numpad7", 103),
        '\u{E022}' => ("8", "Numpad8", 104),
        '\u{E023}' => ("9", "Numpad9", 105),
        '\u{E024}' => ("*", "NumpadMultiply", 106),
        '\u{E025}' => ("+", "NumpadAdd", 107),
        '\u{E026}' => (",", "NumpadComma", 108),
        '\u{E027}' => ("-", "NumpadSubtract", 109),
        '\u{E028}' => (".", "NumpadDecimal", 110),
        '\u{E029}' => ("/", "NumpadDivide", 111),
        '\u{E031}' => ("F1", "F1", 112),
        '\u{E032}' => ("F2", "F2", 113),
        '\u{E033}' => ("F3", "F3", 114),
        '\u{E034}' => ("F4", "F4", 115),
        '\u{E035}' => ("F5", "F5", 116),
        '\u{E036}' => ("F6", "F6", 117),
        '\u{E037}' => ("F7", "F7", 118),
        '\u{E038}' => ("F8", "F8", 119),
        '\u{E039}' => ("F9", "F9", 120),
        '\u{E03A}' => ("F10", "F10", 121),
        '\u{E03B}' => ("F11", "F11", 122),
        '\u{E03C}' => ("F12", "F12", 123),
        '\u{E03D}' => ("Meta", "MetaLeft", 91),
        '\u{E040}' => ("ZenkakuHankaku", "", 0),
        '\u{E050}' => ("Shift", "ShiftRight", 16),
        '\u{E051}' => ("Control", "ControlRight", 17),
        '\u{E052}' => ("Alt", "AltRight", 18),
        '\u{E053}' => ("Meta", "MetaRight", 92),
        '\u{E054}' => ("PageUp", "Numpad9", 33),
        '\u{E055}' => ("PageDown", "Numpad3", 34),
        '\u{E056}' => ("End", "Numpad1", 35),
        '\u{E057}' => ("Home", "Numpad7", 36),
        '\u{E058}' => ("ArrowLeft", "Numpad4", 37),
        '\u{E059}' => ("ArrowUp", "Numpad8", 38),
        '\u{E05A}' => ("ArrowRight", "Numpad6", 39),
        '\u{E05B}' => ("ArrowDown", "Numpad2", 40),
        '\u{E05C}' => ("Insert", "Numpad0", 45),
        '\u{E05D}' => ("Delete", "NumpadDecimal", 46),
        // Code points the spec leaves unassigned still dispatch as a key rather than as text.
        '\u{E000}'..='\u{E05D}' => ("Unidentified", "", 0),
        _ => return None, // printable character
    };
    Some(mapped)
}

/// Whether a normalized key value types a character: Space, `;`, `=` and the numpad digits and
/// operators.
pub fn is_printable_key(js_key: &str) -> bool {
    js_key.chars().count() == 1
}

/// Whether a WebDriver key value is a left- or right-hand Shift/Control/Alt/Meta.
pub fn is_modifier_key(key: &str) -> bool {
    matches!(
        map_special_key(key),
        Some(("Shift" | "Control" | "Alt" | "Meta", _, _))
    )
}

/// The `KeyboardEvent.code` for a printable character.
pub fn regular_key_code_for(key: &str) -> String {
    let ch = key.chars().next().unwrap_or(' ');
    let upper = ch.to_ascii_uppercase();
    if ch.is_ascii_alphabetic() {
        format!("Key{upper}")
    } else if ch.is_ascii_digit() {
        format!("Digit{ch}")
    } else {
        key.to_string()
    }
}

/// A JS string literal for `text` that is safe to splice into a script.
pub fn js_string(text: &str) -> String {
    serde_json::Value::from(text).to_string()
}

/// JS binding `var` to the focused element, looking through open shadow roots.
pub fn deep_active_element_js(var: &str) -> String {
    format!(
        r"var {var} = document.activeElement || document.body;
                while ({var}.shadowRoot && {var}.shadowRoot.activeElement) {{
                    {var} = {var}.shadowRoot.activeElement;
                }}"
    )
}

/// JS expression that is true when `el` takes typed text: a textarea or a text-like input.
pub fn is_text_field_js(el: &str) -> String {
    format!(
        "({el}.tagName === 'TEXTAREA' || ({el}.tagName === 'INPUT' && \
         ['text', 'search', 'url', 'tel', 'email', 'password', 'number'].indexOf({el}.type) !== -1))"
    )
}

/// JS expression that is true when typing may change `el`'s value: a text field that is neither
/// read-only nor disabled.
pub fn is_editable_text_field_js(el: &str) -> String {
    let text_field = is_text_field_js(el);
    format!("({text_field} && !{el}.readOnly && !{el}.disabled)")
}

/// JS expression that is true when `el` is a non-typeable form control whose value Send Keys sets
/// directly (W3C), such as a date or colour input.
pub fn is_value_control_js(el: &str) -> String {
    format!(
        "({el}.tagName === 'INPUT' && \
         ['color', 'date', 'datetime-local', 'month', 'range', 'time', 'week'].indexOf({el}.type) !== -1)"
    )
}

/// JS dispatching a synthetic special-key `KeyboardEvent` plus the editing, radio and line-break
/// side-effects an untrusted event can't drive itself, skipped when a handler cancels the keydown.
pub fn build_special_key_script(
    js_key: &str,
    js_code: &str,
    key_code: u32,
    is_down: bool,
    modifiers: &ModifierState,
) -> String {
    let default_action = match js_key {
        _ if !is_down => String::new(),
        "Backspace" | "Delete" => delete_js(js_key == "Backspace"),
        "ArrowDown" | "ArrowRight" => radio_step_js(true),
        "ArrowUp" | "ArrowLeft" => radio_step_js(false),
        "Enter" if !(modifiers.ctrl || modifiers.meta || modifiers.alt) => {
            line_break_js(modifiers.shift)
        }
        _ => String::new(),
    };
    key_event_script(
        js_key,
        js_code,
        key_code,
        is_down,
        modifiers,
        &default_action,
    )
}

/// JS dispatching a synthetic printable-character `KeyboardEvent` plus the insertion or Ctrl/Meta+A
/// select-all an untrusted event can't drive itself, skipped when a handler cancels the keydown.
pub fn build_regular_key_script(
    key: &str,
    code: &str,
    is_down: bool,
    modifiers: &ModifierState,
) -> String {
    let ch = key.chars().next().unwrap_or(' ');
    build_printable_key_script(key, code, ch as u32, is_down, modifiers)
}

/// [`build_regular_key_script`] with an explicit `keyCode`, for printable special keys whose
/// legacy code differs from the character's (e.g. Numpad5 is 101, not 53).
pub fn build_printable_key_script(
    key: &str,
    code: &str,
    key_code: u32,
    is_down: bool,
    modifiers: &ModifierState,
) -> String {
    let is_select_all = matches!(key, "a" | "A") && (modifiers.ctrl || modifiers.meta);
    let default_action = if !is_down {
        String::new()
    } else if is_select_all {
        select_all_js()
    } else if modifiers.ctrl || modifiers.meta || modifiers.alt {
        String::new()
    } else {
        insert_text_js("activeEl", &js_string(key))
    };
    key_event_script(key, code, key_code, is_down, modifiers, &default_action)
}

/// JS dispatching one keydown or keyup to the deep focused element, then running `default_action`
/// unless a handler canceled the event.
fn key_event_script(
    key: &str,
    code: &str,
    key_code: u32,
    is_down: bool,
    modifiers: &ModifierState,
    default_action: &str,
) -> String {
    let active_element = deep_active_element_js("activeEl");
    let event_type = if is_down { "keydown" } else { "keyup" };
    let key = js_string(key);
    let code = js_string(code);
    let ModifierState {
        ctrl,
        shift,
        alt,
        meta,
    } = *modifiers;
    format!(
        r"(function() {{
                {active_element}
                var keyEvent = new KeyboardEvent('{event_type}', {{
                    key: {key},
                    code: {code},
                    keyCode: {key_code},
                    which: {key_code},
                    ctrlKey: {ctrl},
                    metaKey: {meta},
                    shiftKey: {shift},
                    altKey: {alt},
                    bubbles: true,
                    composed: true,
                    cancelable: true
                }});
                if (!activeEl.dispatchEvent(keyEvent)) {{
                    return true;
                }}
                {default_action}
                return true;
            }})()"
    )
}

/// JS for Backspace (`backward`) or Delete in an editable text field: removes the selection, or the
/// character before or after the caret (the end, without a selection API), and fires `input`.
fn delete_js(backward: bool) -> String {
    let text_field = is_editable_text_field_js("activeEl");
    let (edge_check, removed_start, removed_end, input_type) = if backward {
        (
            "selStart > 0",
            "selStart - 1",
            "selStart",
            "deleteContentBackward",
        )
    } else {
        (
            "selStart < currentValue.length",
            "selStart",
            "selStart + 1",
            "deleteContentForward",
        )
    };
    format!(
        r"if ({text_field}) {{
                    var currentValue = activeEl.value;
                    var selStart = null;
                    var selEnd = null;
                    try {{
                        selStart = activeEl.selectionStart;
                        selEnd = activeEl.selectionEnd;
                    }} catch (e) {{}}
                    var hasSelectionApi = typeof selStart === 'number' && typeof selEnd === 'number';
                    if (!hasSelectionApi) {{
                        // Inputs without a selection API (email, number) edit at the end, as typing appends.
                        selStart = selEnd = currentValue.length;
                    }}
                    var removeStart = selStart;
                    var removeEnd = selEnd;
                    if (selStart === selEnd) {{
                        removeStart = {edge_check} ? {removed_start} : selStart;
                        removeEnd = {edge_check} ? {removed_end} : selStart;
                    }}
                    if (removeStart !== removeEnd) {{
                        var nativeInputValueSetter = Object.getOwnPropertyDescriptor(
                            activeEl.tagName === 'INPUT'
                                ? window.HTMLInputElement.prototype
                                : window.HTMLTextAreaElement.prototype,
                            'value'
                        ).set;
                        nativeInputValueSetter.call(
                            activeEl,
                            currentValue.slice(0, removeStart) + currentValue.slice(removeEnd)
                        );
                        if (hasSelectionApi) {{
                            activeEl.setSelectionRange(removeStart, removeStart);
                        }}
                        activeEl.dispatchEvent(new InputEvent('input', {{
                            bubbles: true,
                            composed: true,
                            cancelable: true,
                            inputType: '{input_type}'
                        }}));
                    }}
                }}"
    )
}

/// JS moving the checked radio to the next (`forward`) or previous button of the focused radio's
/// group, firing `change`.
fn radio_step_js(forward: bool) -> String {
    format!(
        r#"if (activeEl.tagName === 'INPUT' && activeEl.type === 'radio' && activeEl.name) {{
                    var radios = Array.from(activeEl.getRootNode().querySelectorAll(
                        'input[type="radio"][name="' + CSS.escape(activeEl.name) + '"]'
                    ));
                    var currentIndex = radios.indexOf(activeEl);
                    if (currentIndex !== -1 && radios.length > 1) {{
                        var nextIndex = {forward}
                            ? (currentIndex + 1) % radios.length
                            : (currentIndex - 1 + radios.length) % radios.length;
                        var nextRadio = radios[nextIndex];
                        nextRadio.checked = true;
                        nextRadio.focus();
                        nextRadio.dispatchEvent(new Event('change', {{ bubbles: true }}));
                    }}
                }}"#
    )
}

/// JS for Enter: a line break in a textarea or contenteditable (Shift+Enter inserts a `<br>`
/// rather than a new paragraph). Inputs get none.
fn line_break_js(shift: bool) -> String {
    let insert_newline = insert_text_js("activeEl", &js_string("\n"));
    let command = if shift {
        "insertLineBreak"
    } else {
        "insertParagraph"
    };
    format!(
        r"if (activeEl.tagName === 'TEXTAREA') {{
                    {insert_newline}
                }} else if (activeEl.isContentEditable) {{
                    document.execCommand('{command}', false, null);
                }}"
    )
}

/// JS for Ctrl/Meta+A: selects a text field's value, or everything else.
fn select_all_js() -> String {
    let text_field = is_text_field_js("activeEl");
    format!(
        r"if ({text_field}) {{
                    activeEl.select();
                }} else {{
                    document.execCommand('selectAll', false, null);
                }}"
    )
}

/// JS inserting the string expression `text_js` at `el`'s selection, replacing any selected range,
/// and firing `input` if the value changed. Only editable text fields and contenteditable take text.
pub(super) fn insert_text_js(el: &str, text_js: &str) -> String {
    let text_field = is_editable_text_field_js(el);
    format!(
        r"var insertedText = {text_js};
                if ({text_field}) {{
                    var nativeInputValueSetter = Object.getOwnPropertyDescriptor(
                        {el}.tagName === 'INPUT'
                            ? window.HTMLInputElement.prototype
                            : window.HTMLTextAreaElement.prototype,
                        'value'
                    ).set;
                    var currentValue = {el}.value;
                    var selStart = null;
                    var selEnd = null;
                    try {{
                        selStart = {el}.selectionStart;
                        selEnd = {el}.selectionEnd;
                    }} catch (e) {{}}
                    if (typeof selStart === 'number' && typeof selEnd === 'number') {{
                        nativeInputValueSetter.call(
                            {el},
                            currentValue.slice(0, selStart) + insertedText + currentValue.slice(selEnd)
                        );
                        var caret = selStart + insertedText.length;
                        {el}.setSelectionRange(caret, caret);
                    }} else {{
                        nativeInputValueSetter.call({el}, currentValue + insertedText);
                    }}
                    if ({el}.value !== currentValue) {{
                        {el}.dispatchEvent(new InputEvent('input', {{
                            bubbles: true,
                            composed: true,
                            cancelable: true,
                            inputType: 'insertText',
                            data: insertedText
                        }}));
                    }}
                }} else if ({el}.isContentEditable) {{
                    document.execCommand('insertText', false, insertedText);
                }}"
    )
}

#[cfg(test)]
#[path = "key_input_tests/mod.rs"]
mod tests;
