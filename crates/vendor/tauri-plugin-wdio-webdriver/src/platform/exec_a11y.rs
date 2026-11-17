use crate::platform::executor::PlatformExecutor;
use crate::platform::executor_util::extract_string_value;
use crate::server::response::WebDriverErrorResponse;
use async_trait::async_trait;
use tauri::Runtime;

/// Computed accessibility role and label commands.
#[async_trait]
pub trait ElementA11yOps<R: Runtime>: PlatformExecutor<R> {
    /// Get element's computed accessibility role
    async fn get_element_computed_role(
        &self,
        js_var: &str,
    ) -> Result<String, WebDriverErrorResponse> {
        let script = format!(
            r"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}

                // Check for explicit role attribute first
                var explicitRole = el.getAttribute('role');
                if (explicitRole) return explicitRole;

                // Try computedRole if available (Chrome/Edge)
                if (el.computedRole) return el.computedRole;

                // Compute implicit role based on element type
                var tag = el.tagName.toLowerCase();
                var type = el.type ? el.type.toLowerCase() : '';

                // Map elements to their implicit ARIA roles
                var roleMap = {{
                    'a': el.hasAttribute('href') ? 'link' : 'generic',
                    'article': 'article',
                    'aside': 'complementary',
                    'button': 'button',
                    'datalist': 'listbox',
                    'details': 'group',
                    'dialog': 'dialog',
                    'fieldset': 'group',
                    'figure': 'figure',
                    'footer': 'contentinfo',
                    'form': 'form',
                    'h1': 'heading',
                    'h2': 'heading',
                    'h3': 'heading',
                    'h4': 'heading',
                    'h5': 'heading',
                    'h6': 'heading',
                    'header': 'banner',
                    'hr': 'separator',
                    'img': el.getAttribute('alt') === '' ? 'presentation' : 'img',
                    'li': 'listitem',
                    'main': 'main',
                    'menu': 'list',
                    'meter': 'meter',
                    'nav': 'navigation',
                    'ol': 'list',
                    'optgroup': 'group',
                    'option': 'option',
                    'output': 'status',
                    'progress': 'progressbar',
                    'section': 'region',
                    'select': el.multiple ? 'listbox' : 'combobox',
                    'summary': 'button',
                    'table': 'table',
                    'tbody': 'rowgroup',
                    'td': 'cell',
                    'textarea': 'textbox',
                    'tfoot': 'rowgroup',
                    'th': 'columnheader',
                    'thead': 'rowgroup',
                    'tr': 'row',
                    'ul': 'list'
                }};

                // Handle input types
                if (tag === 'input') {{
                    var inputRoles = {{
                        'button': 'button',
                        'checkbox': 'checkbox',
                        'email': 'textbox',
                        'image': 'button',
                        'number': 'spinbutton',
                        'radio': 'radio',
                        'range': 'slider',
                        'reset': 'button',
                        'search': 'searchbox',
                        'submit': 'button',
                        'tel': 'textbox',
                        'text': 'textbox',
                        'url': 'textbox'
                    }};
                    return inputRoles[type] || 'textbox';
                }}

                return roleMap[tag] || '';
            }})()"
        );
        let result = self.evaluate_js(&script).await?;
        extract_string_value(&result)
    }

    /// Get element's computed accessibility label
    async fn get_element_computed_label(
        &self,
        js_var: &str,
    ) -> Result<String, WebDriverErrorResponse> {
        let script = format!(
            r#"(function() {{
                var el = window.{js_var};
                if (!el || !el.isConnected) {{
                    throw new Error('stale element reference');
                }}

                // Try computedName if available (Chrome/Edge)
                if (el.computedName) return el.computedName;

                // Check aria-labelledby first (highest priority)
                var labelledBy = el.getAttribute('aria-labelledby');
                if (labelledBy) {{
                    var labels = labelledBy.split(/\s+/).map(function(id) {{
                        var labelEl = document.getElementById(id);
                        return labelEl ? labelEl.textContent : '';
                    }});
                    var combined = labels.join(' ').trim();
                    if (combined) return combined;
                }}

                // Check aria-label
                var ariaLabel = el.getAttribute('aria-label');
                if (ariaLabel) return ariaLabel;

                // For inputs, check associated label
                var tag = el.tagName.toLowerCase();
                if (tag === 'input' || tag === 'textarea' || tag === 'select') {{
                    // Check for label with 'for' attribute
                    if (el.id) {{
                        var label = document.querySelector("label[for='" + el.id + "']");
                        if (label) return label.textContent.trim();
                    }}
                    // Check for wrapping label
                    var parentLabel = el.closest('label');
                    if (parentLabel) {{
                        // Get label text excluding the input's value
                        var clone = parentLabel.cloneNode(true);
                        var inputs = clone.querySelectorAll('input, textarea, select');
                        inputs.forEach(function(input) {{ input.remove(); }});
                        var labelText = clone.textContent.trim();
                        if (labelText) return labelText;
                    }}
                    // Check placeholder
                    if (el.placeholder) return el.placeholder;
                }}

                // For buttons and links, use text content
                if (tag === 'button' || tag === 'a') {{
                    return el.textContent.trim();
                }}

                // For images, use alt text
                if (tag === 'img') {{
                    return el.getAttribute('alt') || '';
                }}

                // Check title attribute as last resort
                var title = el.getAttribute('title');
                if (title) return title;

                // Fall back to text content for other elements
                return el.textContent ? el.textContent.trim() : '';
            }})()"#
        );
        let result = self.evaluate_js(&script).await?;
        extract_string_value(&result)
    }
}

impl<R: Runtime, T: PlatformExecutor<R> + ?Sized> ElementA11yOps<R> for T {}
