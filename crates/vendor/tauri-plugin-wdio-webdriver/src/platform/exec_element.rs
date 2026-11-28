use crate::platform::exec_actions::ActionOps;
use crate::platform::executor::{ElementRect, PlatformExecutor};
use crate::platform::executor_util::{extract_bool_value, extract_string_value, extract_value};
use crate::server::response::WebDriverErrorResponse;
use async_trait::async_trait;
use serde_json::Value;
use tauri::Runtime;

/// Element state, geometry and interaction commands.
#[async_trait]
pub trait ElementStateOps<R: Runtime>: PlatformExecutor<R> {
    /// Get element text content
    async fn get_element_text(&self, js_var: &str) -> Result<String, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                return el.textContent || '';
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_string_value(&result)
    }

    /// Get element tag name (lowercase)
    async fn get_element_tag_name(&self, js_var: &str) -> Result<String, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                return el.tagName.toLowerCase();
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_string_value(&result)
    }

    /// Get element attribute value
    /// Per W3C `WebDriver` spec, certain attributes should return current property values:
    /// - "value" on input/textarea returns current value property
    /// - "checked" on checkbox/radio returns current checked state
    /// - "selected" on option returns current selected state
    async fn get_element_attribute(
        &self,
        js_var: &str,
        name: &str,
    ) -> Result<Option<String>, WebDriverErrorResponse> {
        let escaped_name = name.replace('\\', "\\\\").replace('\'', "\\'");
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                var attrName = '{escaped_name}'.toLowerCase();
                var tagName = el.tagName.toLowerCase();

                // Per W3C WebDriver spec, return property values for certain attributes
                if (attrName === 'value') {{
                    if (tagName === 'input' || tagName === 'textarea') {{
                        return el.value;
                    }}
                }}
                if (attrName === 'checked') {{
                    if (tagName === 'input' && (el.type === 'checkbox' || el.type === 'radio')) {{
                        return el.checked ? 'true' : null;
                    }}
                }}
                if (attrName === 'selected') {{
                    if (tagName === 'option') {{
                        return el.selected ? 'true' : null;
                    }}
                }}

                return el.getAttribute('{escaped_name}');
            }})()"
        );
        let result = self.evaluate_js(&script).await?;

        if let Some(value) = result.get("value") {
            if value.is_null() {
                return Ok(None);
            }
            if let Some(s) = value.as_str() {
                return Ok(Some(s.to_string()));
            }
        }
        Ok(None)
    }

    /// Get element property value
    async fn get_element_property(
        &self,
        js_var: &str,
        name: &str,
    ) -> Result<Value, WebDriverErrorResponse> {
        let escaped_name = name.replace('\\', "\\\\").replace('\'', "\\'");
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                return el['{escaped_name}'];
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_value(&result)
    }

    /// Get element CSS property value
    async fn get_element_css_value(
        &self,
        js_var: &str,
        property: &str,
    ) -> Result<String, WebDriverErrorResponse> {
        let escaped_prop = property.replace('\\', "\\\\").replace('\'', "\\'");
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                return window.getComputedStyle(el).getPropertyValue('{escaped_prop}');
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_string_value(&result)
    }

    /// Get element bounding rectangle
    async fn get_element_rect(&self, js_var: &str) -> Result<ElementRect, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                var rect = el.getBoundingClientRect();
                return {{
                    x: rect.x + window.scrollX,
                    y: rect.y + window.scrollY,
                    width: rect.width,
                    height: rect.height
                }};
            }})()"
        );
        let result = self.evaluate_js(&script).await?;

        if let Some(value) = result.get("value") {
            return Ok(ElementRect {
                x: value.get("x").and_then(Value::as_f64).unwrap_or(0.0),
                y: value.get("y").and_then(Value::as_f64).unwrap_or(0.0),
                width: value.get("width").and_then(Value::as_f64).unwrap_or(0.0),
                height: value.get("height").and_then(Value::as_f64).unwrap_or(0.0),
            });
        }
        Ok(ElementRect::default())
    }

    /// Get an element's in-view center point in **client (viewport)** coordinates,
    /// scrolling it into view first. Unlike [`Executor::get_element_rect`], this
    /// does not add scroll offsets: pointer events dispatch against viewport
    /// coordinates (`clientX`/`clientY`), so the center must be viewport-relative.
    async fn get_element_center(&self, js_var: &str) -> Result<(i32, i32), WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                el.scrollIntoView({{ behavior: 'instant', block: 'center', inline: 'center' }});
                var r = el.getBoundingClientRect();
                return {{
                    x: Math.floor(r.left + r.width / 2),
                    y: Math.floor(r.top + r.height / 2)
                }};
            }})()"
        );
        let result = self.evaluate_js(&script).await?;

        let value = result.get("value").cloned().ok_or_else(|| {
            WebDriverErrorResponse::unknown_error("element center script returned no value")
        })?;

        #[derive(serde::Deserialize)]
        struct Center {
            x: i32,
            y: i32,
        }
        let center: Center = serde_json::from_value(value).map_err(|err| {
            WebDriverErrorResponse::unknown_error(&format!("could not read element center: {err}"))
        })?;
        Ok((center.x, center.y))
    }

    /// Check if element is displayed
    async fn is_element_displayed(&self, js_var: &str) -> Result<bool, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                var style = window.getComputedStyle(el);
                return style.display !== 'none' && style.visibility !== 'hidden' && el.offsetParent !== null;
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_bool_value(&result)
    }

    /// Check if element is enabled
    async fn is_element_enabled(&self, js_var: &str) -> Result<bool, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                return !el.disabled;
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_bool_value(&result)
    }

    /// Check if element is selected (for checkboxes, radio buttons, options)
    async fn is_element_selected(&self, js_var: &str) -> Result<bool, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                if (el.tagName === 'INPUT' && (el.type === 'checkbox' || el.type === 'radio')) {{
                    return el.checked;
                }}
                if (el.tagName === 'OPTION') {{
                    return el.selected;
                }}
                return false;
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_bool_value(&result)
    }

    /// Click on element
    async fn click_element(&self, js_var: &str) -> Result<(), WebDriverErrorResponse> {
        let active_element = crate::platform::key_input::deep_active_element_js("activeEl");
        let text_field = crate::platform::key_input::is_text_field_js("el");
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                {active_element}
                // A caret set before focus stops WebKit selecting a fresh field's text (or putting a
                // textarea's caret first), while a select-on-focus handler still picks its own selection.
                if (activeEl !== el && {text_field}) {{
                    try {{
                        el.setSelectionRange(el.value.length, el.value.length);
                    }} catch (e) {{}}
                }}
                el.scrollIntoView({{ block: 'center', inline: 'center' }});
                el.click();
                // Explicitly focus the element after click - programmatic click()
                // doesn't always trigger focus like a real click would
                if (typeof el.focus === 'function') {{
                    el.focus();
                }}
                return true;
            }})()"
        );
        self.evaluate_js(&script).await?;
        Ok(())
    }

    /// Clear element content (for inputs/textareas)
    async fn clear_element(&self, js_var: &str) -> Result<(), WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}
                if ((el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') && (el.disabled || el.readOnly)) {{
                    return 'not editable';
                }}
                el.focus();
                if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') {{
                    var nativeInputValueSetter = Object.getOwnPropertyDescriptor(
                        el.tagName === 'INPUT' ? window.HTMLInputElement.prototype : window.HTMLTextAreaElement.prototype,
                        'value'
                    ).set;
                    nativeInputValueSetter.call(el, '');
                    var inputEvent = new InputEvent('input', {{
                        bubbles: true,
                        cancelable: true,
                        inputType: 'deleteContentBackward'
                    }});
                    el.dispatchEvent(inputEvent);
                    var changeEvent = new Event('change', {{ bubbles: true }});
                    el.dispatchEvent(changeEvent);
                }} else if (el.isContentEditable) {{
                    el.innerHTML = '';
                }}
                return true;
            }})()"
        );
        if extract_value(&self.evaluate_js(&script).await?)?.as_str() == Some("not editable") {
            return Err(WebDriverErrorResponse::invalid_element_state(
                "element is disabled or read-only, so it cannot be cleared",
            ));
        }
        Ok(())
    }

    /// Send keys to element
    async fn send_keys_to_element(
        &self,
        js_var: &str,
        text: &str,
    ) -> Result<(), WebDriverErrorResponse> {
        use crate::platform::key_input::{self, SendKeysStep};

        let steps = key_input::plan_send_keys(text);
        let start_script = key_input::build_send_keys_start_script(
            js_var,
            &key_input::typed_text(&steps),
            key_input::plain_text(&steps),
        );
        let start = extract_value(&self.evaluate_js(&start_script).await?)?;
        match start.get("status").and_then(Value::as_str) {
            Some("done" | "set") => return Ok(()),
            Some("focused") => {}
            Some("stale") => return Err(WebDriverErrorResponse::stale_element_reference()),
            Some("immutable") => {
                return Err(WebDriverErrorResponse::element_not_interactable(
                    "form control is read-only or disabled, so its value cannot be set",
                ))
            }
            Some("invalid") => {
                return Err(WebDriverErrorResponse::invalid_argument(
                    "text is not a valid value for this form control",
                ))
            }
            Some("file") => {
                return Err(WebDriverErrorResponse::invalid_argument(
                    "sending keys to a file input (file upload) is not supported",
                ))
            }
            _ => {
                return Err(WebDriverErrorResponse::element_not_interactable(
                    "element did not take keyboard focus, so keys cannot be sent to it",
                ))
            }
        }
        for step in &steps {
            match step {
                SendKeysStep::Text(run) => {
                    self.evaluate_js(&key_input::build_insert_run_script(run))
                        .await?;
                }
                SendKeysStep::Key {
                    value,
                    is_down,
                    modifiers,
                } => self.dispatch_key_event(value, *is_down, modifiers).await?,
            }
        }
        if let Some(initial_value) = start.get("value").and_then(Value::as_str) {
            let change_script = key_input::build_send_keys_change_script(js_var, initial_value);
            self.evaluate_js(&change_script).await?;
        }
        Ok(())
    }

    /// Get the active (focused) element and store in `js_var`
    /// Returns true if an active element was found
    async fn get_active_element(&self, js_var: &str) -> Result<bool, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = document.activeElement;
                if (el && el !== document.body) {{
                    window.{js_var} = el;
                    return true;
                }}
                return false;
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_bool_value(&result)
    }
}

impl<R: Runtime, T: PlatformExecutor<R> + ?Sized> ElementStateOps<R> for T {}
