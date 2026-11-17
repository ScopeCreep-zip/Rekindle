use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
use objc2_foundation::{NSData, NSDictionary, NSError, NSString};
use serde_json::Value;

use crate::platform::PrintOptions;

/// The `callAsyncJavaScript` body for an async WebDriver script: deserialises the
/// W3C arguments (element references included) and appends the done-callback.
/// `callAsyncJavaScript` treats the body as function statements, so `return` is
/// required; without it the Promise is discarded.
pub(super) fn async_script_wrapper(script: &str, args_json: &str) -> String {
    format!(
        r"return new Promise((resolve, reject) => {{
            var ELEMENT_KEY = 'element-6066-11e4-a52e-4f735466cecf';
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
            var __done = function(result, error) {{
                if (error) {{
                    reject(new Error(typeof error === 'string' ? error : String(error)));
                }} else {{
                    resolve(result);
                }}
            }};
            var __args = {args_json}.map(deserializeArg);
            __args.push(__done);
            try {{
                (function() {{ {script} }}).apply(null, __args);
            }} catch (e) {{
                reject(e);
            }}
        }})"
    )
}

/// The script that injects CSS `@page` rules for a W3C Print Page request (WKPDFConfiguration
/// has no page size or margins, so they are set through CSS).
pub(super) fn print_style_script(options: &PrintOptions) -> String {
    let page_width = options.page_width.unwrap_or(21.0);
    let page_height = options.page_height.unwrap_or(29.7);
    let margin_top = options.margin_top.unwrap_or(1.0);
    let margin_bottom = options.margin_bottom.unwrap_or(1.0);
    let margin_left = options.margin_left.unwrap_or(1.0);
    let margin_right = options.margin_right.unwrap_or(1.0);
    let orientation = options.orientation.as_deref().unwrap_or("portrait");

    // Inject @page CSS rules
    format!(
        r"(function() {{
            var style = document.createElement('style');
            style.id = '__webdriver_print_style';
            style.textContent = `
                @page {{
                    size: {page_width}cm {page_height}cm {orientation};
                    margin: {margin_top}cm {margin_right}cm {margin_bottom}cm {margin_left}cm;
                }}
                @media print {{
                    body {{
                        -webkit-print-color-adjust: exact;
                        print-color-adjust: exact;
                    }}
                }}
            `;
            document.head.appendChild(style);
            return true;
        }})()"
    )
}

/// Convert `NSImage` to PNG and encode as base64
pub(super) fn image_to_png_base64(image: &NSImage) -> Result<String, String> {
    let tiff_data: Option<Retained<NSData>> = image.TIFFRepresentation();
    let tiff_data = tiff_data.ok_or("Failed to get TIFF representation")?;

    let bitmap_rep = NSBitmapImageRep::imageRepWithData(&tiff_data)
        .ok_or("Failed to create bitmap image rep")?;

    let empty_dict: Retained<NSDictionary<NSString>> = NSDictionary::new();
    // SAFETY: `representationUsingType:properties:` takes an NSDictionary of
    // NSBitmapImageRepPropertyKey values; an empty dictionary is valid and asks for defaults.
    let png_data: Option<Retained<NSData>> = unsafe {
        bitmap_rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &empty_dict)
    };
    let png_data = png_data.ok_or("Failed to convert to PNG")?;

    let bytes = png_data.to_vec();
    Ok(BASE64_STANDARD.encode(&bytes))
}

/// Build a diagnostic string for an `NSError` returned by a `WKWebView` completion handler.
///
/// `localizedDescription` for a script failure is frequently the opaque
/// "A JavaScript exception occurred." with no detail. WebKit stashes the real
/// exception text (and source location) in `userInfo` under the `WKJavaScriptException*`
/// keys, so surface those when present. Without this, a script failure on the embedded
/// executor is undiagnosable from the WDIO side (all we get back is the opaque string).
pub(super) fn describe_ns_error(error: &NSError) -> String {
    use std::fmt::Write as _;

    let mut out = format!("{} (code {})", error.localizedDescription(), error.code());

    let user_info = error.userInfo();
    let get = |key: &str| -> Option<String> {
        let ns_key = NSString::from_str(key);
        user_info
            .objectForKey(&ns_key)
            .and_then(|v| match ns_object_to_json(&v) {
                Value::String(s) if !s.is_empty() => Some(s),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
    };

    if let Some(message) = get("WKJavaScriptExceptionMessage") {
        out.push_str(": ");
        out.push_str(&message);
    }
    if let (Some(line), Some(column)) = (
        get("WKJavaScriptExceptionLineNumber"),
        get("WKJavaScriptExceptionColumnNumber"),
    ) {
        let _ = write!(out, " @ {line}:{column}");
    }

    out
}

/// Largest magnitude below which every integer is exactly representable in an `f64` (2^53).
const F64_EXACT_INT_LIMIT: f64 = 9_007_199_254_740_992.0;

/// Convert an `NSObject` (a WebKit script result) to a JSON value.
///
/// Dispatch is by class name, then each branch sends only messages that class
/// implements (`boolValue`/`doubleValue`/`longLongValue` on NSNumber, `count` and
/// `objectAtIndex:` on NSArray, `allKeys` and `objectForKey:` on NSDictionary).
pub(super) fn ns_object_to_json(obj: &AnyObject) -> Value {
    let class_name = obj.class().name().to_str().unwrap_or("");

    if class_name.contains("String") {
        // SAFETY: the class is an NSString cluster subclass, so the object is an NSString.
        let ns_str: &NSString =
            unsafe { &*std::ptr::from_ref::<AnyObject>(obj).cast::<NSString>() };
        return Value::String(ns_str.to_string());
    }

    if class_name.contains("Boolean") {
        // SAFETY: `__NSCFBoolean` is an NSNumber, which implements `boolValue`.
        let bool_val: Bool = unsafe { msg_send![obj, boolValue] };
        return Value::Bool(bool_val.as_bool());
    }

    if class_name.contains("Number") {
        // SAFETY: NSNumber implements `doubleValue` and `longLongValue`.
        let (double_val, int_val): (f64, i64) =
            unsafe { (msg_send![obj, doubleValue], msg_send![obj, longLongValue]) };
        // An integral value inside f64's exact-integer range is the same number as its
        // `longLongValue`, so report it as a JSON integer.
        if double_val.fract().abs() < f64::EPSILON && double_val.abs() < F64_EXACT_INT_LIMIT {
            return Value::Number(serde_json::Number::from(int_val));
        }
        return serde_json::Number::from_f64(double_val).map_or(Value::Null, Value::Number);
    }

    if class_name.contains("Array") {
        return Value::Array(ns_array_to_json(obj));
    }

    if class_name.contains("Dictionary") {
        return Value::Object(ns_dictionary_to_json(obj));
    }

    Value::Null
}

/// Borrow a non-null element returned by a Foundation collection accessor.
fn borrow_object<'a>(ptr: *mut AnyObject) -> Option<&'a AnyObject> {
    // SAFETY: the pointer comes from `objectAtIndex:`/`objectForKey:`/`allKeys` on a
    // collection the caller borrows, which keeps the element alive for that borrow.
    unsafe { ptr.as_ref() }
}

fn ns_array_to_json(obj: &AnyObject) -> Vec<Value> {
    // SAFETY: the object is an NSArray, which implements `count`.
    let count: usize = unsafe { msg_send![obj, count] };
    (0..count)
        .filter_map(|i| {
            // SAFETY: `i < count`, so `objectAtIndex:` is in bounds.
            let item: *mut AnyObject = unsafe { msg_send![obj, objectAtIndex: i] };
            borrow_object(item).map(ns_object_to_json)
        })
        .collect()
}

fn ns_dictionary_to_json(obj: &AnyObject) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    // SAFETY: the object is an NSDictionary, which implements `allKeys`.
    let keys_ptr: *mut AnyObject = unsafe { msg_send![obj, allKeys] };
    let Some(keys) = borrow_object(keys_ptr) else {
        return map;
    };
    for key in ns_array_to_keys(keys) {
        if !key.class().name().to_str().unwrap_or("").contains("String") {
            continue;
        }
        // SAFETY: the key's class is an NSString cluster subclass.
        let ns_key: &NSString =
            unsafe { &*std::ptr::from_ref::<AnyObject>(key).cast::<NSString>() };
        // SAFETY: NSDictionary implements `objectForKey:`; the key came from its `allKeys`.
        let val: *mut AnyObject = unsafe { msg_send![obj, objectForKey: key] };
        if let Some(val) = borrow_object(val) {
            map.insert(ns_key.to_string(), ns_object_to_json(val));
        }
    }
    map
}

fn ns_array_to_keys(keys: &AnyObject) -> Vec<&AnyObject> {
    // SAFETY: `allKeys` returns an NSArray, which implements `count`.
    let count: usize = unsafe { msg_send![keys, count] };
    (0..count)
        .filter_map(|i| {
            // SAFETY: `i < count`, so `objectAtIndex:` is in bounds.
            let key: *mut AnyObject = unsafe { msg_send![keys, objectAtIndex: i] };
            borrow_object(key)
        })
        .collect()
}
