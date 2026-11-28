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

        // The script runs as an async function body (W3C WebDriver §13.2.2: the script is a
        // function body applied to the arguments); its result object comes back from the call
        // itself (`call_async_function`), never through a page global a navigation can wipe.
        let body = format!(
            r"
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

                try {{
                    var args = {args_json}.map(deserializeArg);
                    var raw_result = await (async function() {{ {script} }}).apply(null, args);
                    return {{ __wd_success: true, __wd_value: serializeValue(raw_result) }};
                }} catch (e) {{
                    return {{ __wd_success: false, __wd_error: e.message || String(e) }};
                }}",
        );

        let inner = self.call_async_function(&body).await?;
        extract_script_result_from_inner(&inner)
    }
}

impl<R: Runtime, T: PlatformExecutor<R> + ?Sized> ScriptOps<R> for T {}
