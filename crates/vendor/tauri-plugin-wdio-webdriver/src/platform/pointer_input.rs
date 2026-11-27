use super::PointerEventType;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PointerInputState {
    /// DOM `buttons` bitmask of the buttons held once the event applies
    pub buttons: u32,
    /// Set while a canceled `pointerdown` suppresses compatibility mouse events
    pub suppress_mouse_events: bool,
    /// Another button stays held across this press or release, so the pointer
    /// reports it as a `pointermove` instead of `pointerdown`/`pointerup`
    pub chorded: bool,
    /// This primary release lands where its press did, so it also fires click
    pub clicks: bool,
}

/// Script that dispatches one pointer action's events at (`x`, `y`) and returns
/// whether its `pointerdown` was canceled.
pub fn pointer_event_script(
    event_type: PointerEventType,
    x: i32,
    y: i32,
    button: u32,
    input_state: PointerInputState,
) -> String {
    let PointerInputState {
        buttons,
        suppress_mouse_events,
        chorded,
        clicks,
    } = input_state;
    let (pointer_event_name, mouse_event_name) = match event_type {
        PointerEventType::Down if chorded => ("pointermove", "mousedown"),
        PointerEventType::Down => ("pointerdown", "mousedown"),
        PointerEventType::Up if chorded => ("pointermove", "mouseup"),
        PointerEventType::Up => ("pointerup", "mouseup"),
        PointerEventType::Move => ("pointermove", "mousemove"),
    };

    // Pointer Events: a `pointermove` that doesn't change a button reports -1.
    let pointer_button = if matches!(event_type, PointerEventType::Move) {
        -1
    } else {
        i64::from(button)
    };
    let pressure = if buttons == 0 { "0" } else { "0.5" };
    let pointer_init = format!(
        r"{{
            pointerId: 1,
            pointerType: 'mouse',
            isPrimary: true,
            bubbles: true,
            cancelable: true,
            composed: true,
            clientX: {x},
            clientY: {y},
            button: {pointer_button},
            buttons: {buttons},
            pressure: {pressure},
        }}"
    );
    // Only a canceled pointerdown starts suppression, never a chorded pointermove.
    let pointer_dispatch = if pointer_event_name == "pointerdown" {
        format!("pointerDownCanceled = !el.dispatchEvent(new PointerEvent('pointerdown', {pointer_init}));")
    } else {
        format!("el.dispatchEvent(new PointerEvent('{pointer_event_name}', {pointer_init}));")
    };

    let mouse_event = |name: &str| {
        let detail = u8::from(name != "mousemove");
        format!(
            r"new MouseEvent('{name}', {{
                bubbles: true,
                cancelable: true,
                composed: true,
                view: window,
                detail: {detail},
                clientX: {x},
                clientY: {y},
                button: {button},
                buttons: {buttons},
            }})"
        )
    };
    let mouse_dispatch = if suppress_mouse_events {
        String::new()
    } else {
        format!(
            r"if (!pointerDownCanceled) {{
                el.dispatchEvent({});
            }}",
            mouse_event(mouse_event_name)
        )
    };

    let is_primary_button = button == 0;
    // The press's ancestor path lets click find the nearest ancestor of the press target
    // that is still connected and contains the release target, as browsers do.
    let press_path = if matches!(event_type, PointerEventType::Down) && is_primary_button {
        r"var path = [];
            for (var node = el; node; node = node.parentNode) {
                path.push(node);
            }
            window.__wdioPointerDownPath = path;"
            .to_string()
    } else {
        String::new()
    };
    // click is not a compatibility mouse event, so suppression never drops it.
    let click_dispatch = if clicks {
        format!(
            r"var path = window.__wdioPointerDownPath || [];
            for (var i = 0; i < path.length; i++) {{
                if (path[i].isConnected && path[i].contains(el)) {{
                    path[i].dispatchEvent({});
                    break;
                }}
            }}",
            mouse_event("click")
        )
    } else {
        String::new()
    };
    format!(
        r"(function() {{
            var el = document.elementFromPoint({x}, {y});
            if (!el) {{
                el = document.body;
            }}
            {press_path}
            var pointerDownCanceled = false;
            {pointer_dispatch}
            {mouse_dispatch}
            {click_dispatch}
            return pointerDownCanceled;
        }})()"
    )
}

#[cfg(test)]
#[path = "pointer_input_tests.rs"]
mod tests;
