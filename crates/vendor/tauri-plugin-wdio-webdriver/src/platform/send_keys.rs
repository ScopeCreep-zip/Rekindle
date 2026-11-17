//! Element Send Keys planning (W3C WebDriver §12.4.5): text runs, special keys and
//! held modifiers, and the scripts that type them.

use std::collections::BTreeSet;

use super::executor::ModifierState;
use super::key_input::{
    deep_active_element_js, insert_text_js, is_modifier_key, is_text_field_js, is_value_control_js,
    js_string, map_special_key, NULL_KEY,
};

/// One segment of an element send-keys request: a run of printable text, or a single special key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySegment {
    Text(String),
    Special(String),
}

/// Split send-keys text into printable runs and individual special keys, preserving order.
pub fn segment_send_keys(text: &str) -> Vec<KeySegment> {
    let mut segments = Vec::new();
    let mut run = String::new();
    for ch in text.chars() {
        if map_special_key(&ch.to_string()).is_some() {
            if !run.is_empty() {
                segments.push(KeySegment::Text(std::mem::take(&mut run)));
            }
            segments.push(KeySegment::Special(ch.to_string()));
        } else {
            run.push(ch);
        }
    }
    if !run.is_empty() {
        segments.push(KeySegment::Text(run));
    }
    segments
}

/// The modifier keys held down. Each key is tracked on its own, so releasing right Control leaves
/// Control set while left Control is still held.
#[derive(Debug, Clone, Default)]
pub struct HeldModifiers {
    keys: BTreeSet<String>,
}

impl HeldModifiers {
    /// The modifiers among `keys`, e.g. a session's keys still pressed from earlier actions.
    pub fn from_keys<'a>(keys: impl IntoIterator<Item = &'a String>) -> Self {
        let mut held = Self::default();
        for key in keys {
            held.press(key);
        }
        held
    }

    /// Record a key press; returns whether it newly held a modifier.
    pub fn press(&mut self, key: &str) -> bool {
        is_modifier_key(key) && self.keys.insert(key.to_string())
    }

    /// Record a key release; returns whether it released a held modifier.
    pub fn release(&mut self, key: &str) -> bool {
        self.keys.remove(key)
    }

    /// The modifier flags for the keys still held.
    pub fn state(&self) -> ModifierState {
        let mut state = ModifierState::default();
        for key in &self.keys {
            match map_special_key(key) {
                Some(("Control", _, _)) => state.ctrl = true,
                Some(("Shift", _, _)) => state.shift = true,
                Some(("Alt", _, _)) => state.alt = true,
                Some(("Meta", _, _)) => state.meta = true,
                _ => {}
            }
        }
        state
    }

    /// The held modifier keys in code-point order.
    pub fn keys(&self) -> Vec<String> {
        self.keys.iter().cloned().collect()
    }
}

/// One step of an element send-keys request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendKeysStep {
    /// Printable text inserted into the element.
    Text(String),
    /// A key event for the focused element, dispatched as the actions path dispatches it.
    Key {
        value: String,
        is_down: bool,
        modifiers: ModifierState,
    },
}

/// Send-keys text as ordered steps: a modifier stays held until it's pressed again, the null key or
/// the end, and text typed under Control, Alt or Meta is pressed key by key as shortcuts.
pub fn plan_send_keys(text: &str) -> Vec<SendKeysStep> {
    let mut steps = Vec::new();
    let mut held = HeldModifiers::default();

    for segment in segment_send_keys(text) {
        let modifiers = held.state();
        match segment {
            KeySegment::Text(run) if modifiers.ctrl || modifiers.alt || modifiers.meta => {
                for ch in run.chars() {
                    push_key_press(&mut steps, &ch.to_string(), modifiers);
                }
            }
            KeySegment::Text(run) => push_text(&mut steps, &run),
            KeySegment::Special(key) if key == NULL_KEY => {
                release_held_modifiers(&mut steps, &mut held);
            }
            KeySegment::Special(key) if is_modifier_key(&key) => {
                let is_down = held.press(&key);
                if !is_down {
                    held.release(&key);
                }
                steps.push(SendKeysStep::Key {
                    value: key,
                    is_down,
                    modifiers: held.state(),
                });
            }
            KeySegment::Special(key) => push_key_press(&mut steps, &key, modifiers),
        }
    }
    release_held_modifiers(&mut steps, &mut held);
    steps
}

/// The text a plan types, without its key presses: what a non-typeable form control's value is set to.
pub fn typed_text(steps: &[SendKeysStep]) -> String {
    steps
        .iter()
        .filter_map(|step| match step {
            SendKeysStep::Text(run) => Some(run.as_str()),
            SendKeysStep::Key { .. } => None,
        })
        .collect()
}

fn push_text(steps: &mut Vec<SendKeysStep>, run: &str) {
    if let Some(SendKeysStep::Text(last)) = steps.last_mut() {
        last.push_str(run);
    } else {
        steps.push(SendKeysStep::Text(run.to_string()));
    }
}

fn push_key_press(steps: &mut Vec<SendKeysStep>, value: &str, modifiers: ModifierState) {
    for is_down in [true, false] {
        steps.push(SendKeysStep::Key {
            value: value.to_string(),
            is_down,
            modifiers,
        });
    }
}

/// Key-up every held modifier in code-point order, as the spec's "clear the modifier key state"
/// releases them. Each key-up carries the flags left after its own release.
fn release_held_modifiers(steps: &mut Vec<SendKeysStep>, held: &mut HeldModifiers) {
    for key in held.keys() {
        held.release(&key);
        steps.push(SendKeysStep::Key {
            value: key,
            is_down: false,
            modifiers: held.state(),
        });
    }
}

/// The text to type when a plan presses no keys, so the whole request fits in one script.
pub fn plain_text(steps: &[SendKeysStep]) -> Option<&str> {
    match steps {
        [] => Some(""),
        [SendKeysStep::Text(text)] => Some(text),
        _ => None,
    }
}

/// JS starting send keys: sets a value control's value to `typed`, or focuses the element once and,
/// for a `plain` request, types it too. Returns `{ status, value }` and never throws.
pub fn build_send_keys_start_script(js_var: &str, typed: &str, plain: Option<&str>) -> String {
    let active_element = deep_active_element_js("activeEl");
    let text_field = is_text_field_js("el");
    let receiver_text_field = is_text_field_js("receiver");
    let value_control = is_value_control_js("el");
    let typed = js_string(typed);
    let finish = match plain {
        None => format!(
            r"window.{js_var}__receiver = receiver;
                return {{ status: 'focused', value: initialValue }};"
        ),
        Some(text) => {
            let insert_text = if text.is_empty() {
                String::new()
            } else {
                insert_text_js("receiver", &js_string(text))
            };
            format!(
                r"{insert_text}
                if (initialValue !== null && receiver.value !== initialValue) {{
                    receiver.dispatchEvent(new Event('change', {{ bubbles: true }}));
                }}
                return {{ status: 'done' }};"
            )
        }
    };
    format!(
        r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    return {{ status: 'stale' }};
                }}
                if (el.tagName === 'INPUT' && el.type === 'file') {{
                    return {{ status: 'file' }};
                }}
                var typed = {typed};
                if ({value_control} && typed !== '') {{
                    if (el.readOnly || el.disabled) {{
                        return {{ status: 'immutable' }};
                    }}
                    el.focus();
                    var setValue = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
                    var previousValue = el.value;
                    setValue.call(el, typed);
                    if (el.value === '') {{
                        setValue.call(el, previousValue);
                        return {{ status: 'invalid' }};
                    }}
                    if (el.value !== previousValue) {{
                        el.dispatchEvent(new Event('input', {{ bubbles: true, composed: true }}));
                        el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                    }}
                    return {{ status: 'set' }};
                }}
                {active_element}
                var wasFocused = activeEl === el;
                // Text inside a contenteditable is focused through its editing host.
                var focusTarget = el;
                while (focusTarget.isContentEditable && focusTarget.parentElement
                    && focusTarget.parentElement.isContentEditable) {{
                    focusTarget = focusTarget.parentElement;
                }}
                // Text fields get the caret at the end before focus: typing appends, yet a select-on-focus
                // handler still wins.
                if ({text_field}) {{
                    try {{
                        el.setSelectionRange(el.value.length, el.value.length);
                    }} catch (e) {{}}
                }}
                focusTarget.focus();
                if (!wasFocused && el.isContentEditable) {{
                    try {{
                        var range = document.createRange();
                        range.selectNodeContents(el);
                        range.collapse(false);
                        var selection = window.getSelection();
                        selection.removeAllRanges();
                        selection.addRange(range);
                    }} catch (e) {{}}
                }}
                {active_element}
                // The body, focus-delegating hosts and an editing host holding the element count as focus.
                var focused = el === document.body || el === document.documentElement || activeEl === focusTarget;
                for (var node = activeEl; node && !focused; node = node.parentNode || node.host) {{
                    focused = node === el;
                }}
                if (!focused) {{
                    return {{ status: 'unfocused' }};
                }}
                var receiver = activeEl;
                // A field that focus was delegated to is only known now, so its caret moves after focus.
                if (receiver !== el && {receiver_text_field}) {{
                    try {{
                        receiver.setSelectionRange(receiver.value.length, receiver.value.length);
                    }} catch (e) {{}}
                }}
                var initialValue = {receiver_text_field} ? receiver.value : null;
                {finish}
            }})()"
    )
}

/// JS inserting a printable run at the focused element's selection, as typing does, so text follows
/// focus when a key moves it.
pub fn build_insert_run_script(run: &str) -> String {
    let active_element = deep_active_element_js("activeEl");
    let insert_text = insert_text_js("activeEl", &js_string(run));
    format!(
        r"(function() {{
                {active_element}
                {insert_text}
                return true;
            }})()"
    )
}

/// JS firing one `change` if the field that received the text no longer holds `initial_value`; it
/// skips a detached field so a retry can't duplicate keystrokes that already committed.
pub fn build_send_keys_change_script(js_var: &str, initial_value: &str) -> String {
    let initial_value = js_string(initial_value);
    format!(
        r"(function() {{
                var el = window.{js_var}__receiver;
                delete window.{js_var}__receiver;
                if (el && el.isConnected && el.value !== {initial_value}) {{
                    el.dispatchEvent(new Event('change', {{ bubbles: true }}));
                }}
                return true;
            }})()"
    )
}
