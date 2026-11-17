use crate::platform::executor::PlatformExecutor;
use crate::platform::executor_util::extract_script_result_from_inner;
use crate::server::response::WebDriverErrorResponse;
use async_trait::async_trait;
use serde_json::Value;
use tauri::Runtime;

/// Synchronous script execution with W3C argument (de)serialisation.
#[async_trait]
pub trait ScriptOps<R: Runtime>: PlatformExecutor<R> {
    // =========================================================================
    // Script Execution
    // =========================================================================

    /// Execute synchronous JavaScript with arguments
    async fn execute_script(
        &self,
        script: &str,
        args: &[Value],
    ) -> Result<Value, WebDriverErrorResponse> {
        let args_json = serde_json::to_string(args)
            .map_err(|e| WebDriverErrorResponse::invalid_argument(&e.to_string()))?;

        // Generate unique result variable name
        let result_var = format!("__wdio_exec_{}", uuid::Uuid::new_v4());

        // Wrapper script that:
        // 1. Executes the user's script as a function body (per W3C WebDriver spec §13.2.2)
        // 2. Stores result in a global variable for polling
        // Note: We use an IIFE that returns `undefined` to avoid Promise serialization issues
        //
        // The script is treated as a function body. Clients that want to return a value must
        // include an explicit `return` statement — this matches WebdriverIO's function-object
        // wrapping (`return (fn).apply(null, arguments)`) and raw string scripts like
        // `"return document.title"`.
        let wrapper = format!(
            r"(function() {{
                var ELEMENT_KEY = 'element-6066-11e4-a52e-4f735466cecf';

                function serializeValue(value) {{
                    if (value === null || value === undefined) return null;
                    if (typeof value === 'boolean') return value;
                    if (typeof value === 'number') {{
                        if (!isFinite(value)) return null;
                        return value;
                    }}
                    if (typeof value === 'string') return value;
                    if (typeof value === 'function') return null;
                    if (typeof value === 'symbol') return null;
                    if (typeof value === 'bigint') return Number(value);
                    if (Array.isArray(value)) {{
                        return value.map(serializeValue);
                    }}
                    if (typeof value === 'object') {{
                        if (value[ELEMENT_KEY]) return value;
                        if (value && value.nodeType && value.nodeType === 1) return null;
                        var result = {{}};
                        for (var key in value) {{
                            if (value.hasOwnProperty(key)) {{
                                result[key] = serializeValue(value[key]);
                            }}
                        }}
                        return result;
                    }}
                    return null;
                }}

                function deserializeArg(arg) {{
                    if (arg === null || arg === undefined) return arg;
                    if (Array.isArray(arg)) return arg.map(deserializeArg);
                    if (typeof arg === 'object') {{
                        if (arg[ELEMENT_KEY]) {{
                            var el = window['__wd_el_' + arg[ELEMENT_KEY].replace(/-/g, '')];
                            if (!el) throw new Error('stale element reference');
                            return el;
                        }}
                        var result = {{}};
                        for (var key in arg) {{
                            if (arg.hasOwnProperty(key)) result[key] = deserializeArg(arg[key]);
                        }}
                        return result;
                    }}
                    return arg;
                }}

                // Start async execution (fire and forget)
                (async function() {{
                    try {{
                        var args = {args_json}.map(deserializeArg);
                        // W3C-compliant: wrap as function body, apply with args
                        var raw_result = await (async function() {{ {script} }}).apply(null, args);
                        var serialized = serializeValue(raw_result);
                        window['{result_var}'] = {{ __wd_success: true, __wd_value: serialized }};
                    }} catch (e) {{
                        window['{result_var}'] = {{ __wd_success: false, __wd_error: e.message || String(e) }};
                    }}
                }})();

                // Return undefined to avoid Promise serialization issues
                return undefined;
            }})()",
        );

        // Execute the wrapper script (fire and forget, returns undefined)
        self.evaluate_js(&wrapper).await?;

        // Poll for the result with timeout
        let poll_script = format!("window['{result_var}']");
        let timeout = std::time::Duration::from_millis(self.script_timeout_ms());
        let start = std::time::Instant::now();
        let poll_interval = std::time::Duration::from_millis(50);

        loop {
            let poll_result = self.evaluate_js(&poll_script).await?;
            let inner = poll_result.get("value").cloned().unwrap_or(Value::Null);

            // Check if we have a result
            if !inner.is_null() && inner.get("__wd_success").is_some() {
                // Clean up the global variable
                let cleanup_script = format!("delete window['{result_var}']");
                let _ = self.evaluate_js(&cleanup_script).await;

                return extract_script_result_from_inner(&inner);
            }

            if start.elapsed() > timeout {
                // Clean up on timeout
                let cleanup_script = format!("delete window['{result_var}']");
                let _ = self.evaluate_js(&cleanup_script).await;

                return Err(WebDriverErrorResponse::script_timeout());
            }

            tokio::time::sleep(poll_interval).await;
        }
    }
}

impl<R: Runtime, T: PlatformExecutor<R> + ?Sized> ScriptOps<R> for T {}
