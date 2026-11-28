use crate::platform::executor::{ModifierState, PlatformExecutor, PointerEventType};
use crate::platform::executor_util::extract_bool_value;
use crate::platform::pointer_input::{pointer_event_script, PointerInputState};
use crate::server::response::WebDriverErrorResponse;
use async_trait::async_trait;
use tauri::Runtime;

/// Keyboard, pointer and wheel action dispatch.
#[async_trait]
pub trait ActionOps<R: Runtime>: PlatformExecutor<R> {
    // =========================================================================
    // Actions (Keyboard/Pointer)
    // =========================================================================

    /// Dispatch an Actions key natively when it participates in focus or activation.
    async fn dispatch_action_key(
        &self,
        key: &str,
        is_down: bool,
        modifiers: &ModifierState,
    ) -> Result<(), WebDriverErrorResponse> {
        if let Some(native) = super::NativeKey::from_action(key) {
            self.dispatch_native_key(native, is_down, *modifiers).await
        } else {
            self.dispatch_key_event(key, is_down, modifiers).await
        }
    }

    /// Dispatch a keyboard event with modifier state
    async fn dispatch_key_event(
        &self,
        key: &str,
        is_down: bool,
        modifiers: &ModifierState,
    ) -> Result<(), WebDriverErrorResponse> {
        match crate::platform::key_input::map_special_key(key) {
            Some((js_key, js_code, key_code))
                if crate::platform::key_input::is_printable_key(js_key) =>
            {
                let script = crate::platform::key_input::build_printable_key_script(
                    js_key, js_code, key_code, is_down, modifiers,
                );
                self.evaluate_js(&script).await?;
                Ok(())
            }
            Some((js_key, js_code, key_code)) => {
                let script = crate::platform::key_input::build_special_key_script(
                    js_key, js_code, key_code, is_down, modifiers,
                );
                self.evaluate_js(&script).await?;
                Ok(())
            }
            None => {
                let code = crate::platform::key_input::regular_key_code_for(key);
                self.dispatch_regular_key(key, &code, is_down, modifiers)
                    .await
            }
        }
    }

    /// Dispatch a regular (non-special) key event with modifier state
    async fn dispatch_regular_key(
        &self,
        key: &str,
        code: &str,
        is_down: bool,
        modifiers: &ModifierState,
    ) -> Result<(), WebDriverErrorResponse> {
        let script =
            crate::platform::key_input::build_regular_key_script(key, code, is_down, modifiers);
        self.evaluate_js(&script).await?;
        Ok(())
    }

    /// Dispatch a pointer/mouse event. Returns whether its `pointerdown` was
    /// canceled (a listener called `preventDefault()`).
    async fn dispatch_pointer_event(
        &self,
        event_type: PointerEventType,
        x: i32,
        y: i32,
        button: u32,
        input_state: PointerInputState,
    ) -> Result<bool, WebDriverErrorResponse> {
        let script = pointer_event_script(event_type, x, y, button, input_state);
        let result = self.evaluate_js(&script).await?;
        extract_bool_value(&result)
    }

    /// Dispatch a scroll/wheel event
    async fn dispatch_scroll_event(
        &self,
        x: i32,
        y: i32,
        delta_x: i32,
        delta_y: i32,
    ) -> Result<(), WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = document.elementFromPoint({x}, {y});
                if (!el) el = document.body;

                var event = new WheelEvent('wheel', {{
                    bubbles: true,
                    cancelable: true,
                    clientX: {x},
                    clientY: {y},
                    deltaX: {delta_x},
                    deltaY: {delta_y},
                    deltaMode: 0
                }});
                el.dispatchEvent(event);

                window.scrollBy({delta_x}, {delta_y});
                return true;
            }})()"
        );

        self.evaluate_js(&script).await?;
        Ok(())
    }
}

impl<R: Runtime, T: PlatformExecutor<R> + ?Sized> ActionOps<R> for T {}
