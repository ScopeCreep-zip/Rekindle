//! What the JSON-shape harnesses share (`event_json_shape`,
//! `dto_json_shape`): the camelCase key check and the golden-file compare
//! (`UPDATE_GOLDEN=1` rewrites the file after a deliberate wire change).

use std::path::Path;

use serde_json::{Map, Value};

fn is_camel_case(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase()) && chars.all(|c| c.is_ascii_alphanumeric())
}

/// Every object key under `value` that is not camelCase, by path.
pub fn collect_bad_keys(value: &Value, path: &str, bad: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                let here = format!("{path}.{k}");
                if !is_camel_case(k) {
                    bad.push(here.clone());
                }
                collect_bad_keys(v, &here, bad);
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                collect_bad_keys(v, &format!("{path}[{i}]"), bad);
            }
        }
        _ => {}
    }
}

/// `actual` must equal the golden file at `path`; the panic names each
/// sample added, changed or removed. `what` names the shapes in it.
pub fn check_golden(path: &Path, actual: &Map<String, Value>, what: &str) {
    let mut rendered =
        serde_json::to_string_pretty(&Value::Object(actual.clone())).expect("render golden");
    rendered.push('\n');
    if std::env::var_os("UPDATE_GOLDEN").is_some_and(|v| v == "1") {
        std::fs::write(path, rendered).expect("write golden file");
        return;
    }
    let on_disk = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("read {}: {e} (run with UPDATE_GOLDEN=1)", path.display()));
    if on_disk == rendered {
        return;
    }
    let golden: Map<String, Value> = serde_json::from_str(&on_disk).expect("golden file is JSON");
    let mut report = Vec::new();
    for (id, value) in actual {
        match golden.get(id) {
            None => report.push(format!("added:   {id}")),
            Some(old) if old != value => {
                report.push(format!("changed: {id}\n  golden: {old}\n  actual: {value}"));
            }
            Some(_) => {}
        }
    }
    for id in golden.keys().filter(|id| !actual.contains_key(*id)) {
        report.push(format!("removed: {id}"));
    }
    if report.is_empty() {
        report.push("formatting differs (re-run with UPDATE_GOLDEN=1)".to_owned());
    }
    panic!(
        "{what} JSON differs from {}:\n{}",
        path.display(),
        report.join("\n")
    );
}
